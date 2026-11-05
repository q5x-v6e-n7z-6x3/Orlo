//! GPUI terminal presentation, keyboard routing, clipboard, and IME composition.

#[path = "search.rs"]
mod search;
pub use search::{
    CloseSearch, FindOutput, NextMatch, PreviousMatch, ResumeSearch, ToggleSearchCase, ToggleSearchRegex,
};

use crate::chrome::theme::{self, c};
use gpui::{
    div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle,
    Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, FontStyle, FontWeight,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyBinding, KeyDownEvent, Keystroke, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, ScrollWheelEvent, ShapedLine,
    Style, Task, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use orlo_terminal::mouse::{self, Kind as MouseKind};
use orlo_terminal::{Color, TerminalSession};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, gpui::Action, serde::Deserialize)]
#[action(namespace = terminal, no_json)]
struct SendKey {
    key: String,
}

pub fn key_bindings() -> Vec<KeyBinding> {
    let named = "enter backspace delete tab escape up down left right home end pageup pagedown \
                 insert f1 f2 f3 f4 f5 f6 f7 f8 f9 f10 f11 f12";
    let modifiers = ["", "shift-", "alt-", "ctrl-", "ctrl-shift-", "alt-shift-", "ctrl-alt-", "ctrl-alt-shift-"];
    let named = (named.split_whitespace()).flat_map(|key| modifiers.map(|m| format!("{m}{key}")));
    let controls = ("abcdefghijklmnopqrstuvwxyz@[]\\^_".chars().map(String::from))
        .chain(["space", "2", "6", "-"].map(String::from))
        .map(|key| format!("ctrl-{key}"));
    let mut bindings = vec![
        KeyBinding::new("cmd-c", crate::editor::Copy, Some("Terminal")),
        KeyBinding::new("cmd-v", crate::editor::Paste, Some("Terminal")),
        KeyBinding::new("cmd-a", crate::editor::SelectAll, Some("Terminal")),
    ];
    for context in [Some("Terminal"), Some("TerminalSearch > Editor")] {
        bindings.extend([
            KeyBinding::new("cmd-f", FindOutput, context),
            KeyBinding::new("cmd-g", NextMatch, context),
            KeyBinding::new("cmd-shift-g", PreviousMatch, context),
        ]);
    }
    bindings.extend([
        KeyBinding::new("escape", CloseSearch, Some("TerminalSearch > Editor")),
        KeyBinding::new("enter", NextMatch, Some("TerminalSearch > Editor")),
        KeyBinding::new("shift-enter", PreviousMatch, Some("TerminalSearch > Editor")),
        KeyBinding::new("cmd-alt-c", ToggleSearchCase, Some("TerminalSearch > Editor")),
        KeyBinding::new("cmd-alt-r", ToggleSearchRegex, Some("TerminalSearch > Editor")),
        KeyBinding::new("cmd-alt-u", ResumeSearch, Some("TerminalSearch > Editor")),
    ]);
    bindings
        .extend(named.chain(controls).map(|key| KeyBinding::new(&key, SendKey { key: key.clone() }, Some("Terminal"))));
    bindings
}

fn key_bytes(key: &Keystroke, application_cursor: bool) -> Option<Vec<u8>> {
    let modifiers = key.modifiers;
    orlo_terminal::key_bytes(
        orlo_terminal::Key {
            name: &key.key,
            modifiers: orlo_terminal::Modifiers {
                shift: modifiers.shift,
                alt: modifiers.alt,
                control: modifiers.control,
                platform: modifiers.platform,
            },
        },
        application_cursor,
    )
}

pub struct MetadataChanged;
impl gpui::EventEmitter<MetadataChanged> for Terminal {}

struct CommandReview {
    id: u64,
    screen: orlo_terminal::Screen,
    command: orlo_terminal::commands::Command,
    description: String,
}

pub struct Terminal {
    focus_handle: FocusHandle,
    backend: TerminalSession,
    _poll: Option<Task<()>>,
    /// Reads in a row that found no output: past `QUIET_DRAINS` the shell is read less often.
    quiet: usize,
    /// Scrolled lines not yet whole, so slow trackpad scrolling adds up.
    scroll_rest: f32,
    pub cwd: PathBuf,
    pub(super) name: Option<String>,
    marked: String,
    marked_selection: Range<usize>,
    layout: Option<TerminalLayout>,
    dragging: bool,
    search: Option<search::Search>,
    retained_output: Option<orlo_terminal::search::SearchResults>,
    command_review: Option<CommandReview>,
    reporting_button: Option<MouseButton>,
    last_mouse: Option<(u16, u16)>,
    _blur: Option<gpui::Subscription>,
}

/// How often the shell's output is read, and how often once it has been quiet for
/// `QUIET_DRAINS` reads (a keystroke goes back to `POLL`).
const POLL: Duration = Duration::from_millis(16);
const QUIET_POLL: Duration = Duration::from_millis(200);
const QUIET_DRAINS: usize = 30;

/// Whole lines of `rest` plus a scroll of `lines`, rounded as a single wheel step is; the
/// remainder stays in `rest`.
fn scroll_steps(rest: &mut f32, lines: f32) -> isize {
    *rest += lines;
    let whole = rest.round();
    *rest -= whole;
    whole as isize
}

#[derive(Clone, Copy)]
struct TerminalLayout {
    bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
}

impl Terminal {
    pub fn new(cwd: PathBuf, cx: &mut Context<Self>) -> Self {
        let settings =
            cx.try_global::<crate::config::settings::Settings>().map(|s| s.terminal.clone()).unwrap_or_default();
        let profile = settings.default_profile.as_ref().and_then(|name| settings.profiles.get(name));
        Self::with_profile(cwd, profile, cx)
    }

    pub(super) fn with_profile(
        cwd: PathBuf,
        profile: Option<&crate::config::settings::TerminalProfile>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings =
            cx.try_global::<crate::config::settings::Settings>().map(|s| s.terminal.clone()).unwrap_or_default();
        let cwd = orlo_terminal::shell::startup_directory(profile.and_then(|p| p.directory.as_deref()), &cwd);
        let (shell, arguments, scrollback) = profile
            .map_or((settings.shell.as_deref(), settings.arguments.as_deref(), settings.scrollback), |p| {
                (Some(p.shell.as_str()), Some(p.arguments.as_slice()), p.scrollback)
            });
        let command = orlo_terminal::shell::configured_command(&cwd, shell, arguments);
        let backend = TerminalSession::from_command_with_integration(
            command,
            orlo_terminal::TerminalConfiguration { scrollback, ..Default::default() },
            settings.shell_integration,
        );
        let poll = backend.pty().map(|_| Self::polling(cx));
        Self {
            focus_handle: cx.focus_handle(),
            backend,
            _poll: poll,
            quiet: 0,
            scroll_rest: 0.,
            cwd,
            name: None,
            marked: String::new(),
            marked_selection: 0..0,
            layout: None,
            dragging: false,
            search: None,
            retained_output: None,
            command_review: None,
            reporting_button: None,
            last_mouse: None,
            _blur: None,
        }
    }

    pub(super) fn scrollback_limit(&self) -> usize {
        self.backend.screen().scrollback_limit()
    }

    pub(super) fn set_scrollback_limit(&mut self, rows: usize, cx: &mut Context<Self>) {
        if self.backend.set_scrollback_limit(rows) {
            self.retention_changed(cx);
        }
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    pub(super) fn close(&mut self) {
        self._poll = None;
        self.search = None;
        self.backend.close();
    }

    /// Read the shell's output after each delay `drain` gives, until it exits.
    fn polling(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |terminal, cx| {
            let mut delay = POLL;
            loop {
                cx.background_executor().timer(delay).await;
                match terminal.update(cx, |terminal, cx| terminal.drain(cx)) {
                    Ok(Some(next)) => delay = next,
                    _ => break,
                }
            }
        })
    }

    /// The shell's output so far; the delay before the next read while it runs.
    fn drain(&mut self, cx: &mut Context<Self>) -> Option<Duration> {
        let was_ready = self.can_sync_folder();
        let status = self.backend.drain();
        if !status.running {
            self.reporting_button = None;
            self.last_mouse = None;
        }
        let cwd = self.backend.working_directory().to_path_buf();
        if self.cwd != cwd || was_ready != self.can_sync_folder() || !status.running && status.changed {
            self.cwd = cwd;
            cx.emit(MetadataChanged);
        }
        if status.changed {
            cx.notify();
            self.quiet = 0;
        } else {
            self.quiet = self.quiet.saturating_add(1);
        }
        self.tick_search(status.changed, cx);
        if !status.running && status.changed {
            self.refresh_search(false, cx);
        }
        let delay = if self.quiet < QUIET_DRAINS { POLL } else { QUIET_POLL };
        status.running.then_some(delay)
    }

    /// Input is on its way: read the reply at full speed, now rather than after a quiet
    /// delay.
    fn wake(&mut self, cx: &mut Context<Self>) {
        if self.quiet >= QUIET_DRAINS && self._poll.is_some() && self.backend.exited().is_none() {
            self._poll = Some(Self::polling(cx));
        }
        self.quiet = 0;
    }

    fn write(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let was_ready = self.can_sync_folder();
        self.backend.write(bytes);
        self.wake(cx);
        if was_ready != self.can_sync_folder() {
            cx.emit(MetadataChanged);
        }
        if self.backend.error().is_none() {
            self.search = None;
            self.retained_output = None;
            self.command_review = None;
        }
        cx.notify();
    }

    pub fn review_command(&mut self, forward: bool, cx: &mut Context<Self>) {
        let records = self.backend.commands().records();
        if records.is_empty() {
            return;
        }
        let current = self.command_review.as_ref().and_then(|r| records.iter().position(|c| c.id == r.id));
        let index = match current {
            Some(i) if forward && i + 1 == records.len() => {
                self.return_to_live(cx);
                return;
            }
            Some(i) => {
                if forward {
                    i + 1
                } else {
                    i.saturating_sub(1)
                }
            }
            None => records.len() - 1,
        };
        let command = &records[index];
        let (rows, cols) = self.backend.screen().size();
        let screen = command.review_screen(rows, cols);
        let description = format!(
            "#{} · exit {} · {}",
            command.id,
            command.status.map_or("unknown".into(), |n| n.to_string()),
            command.command
        );
        self.command_review = Some(CommandReview { id: command.id, screen, command: command.clone(), description });
        self.search = None;
        self.retained_output = None;
        self.backend.set_selection(None);
        cx.notify();
    }

    pub fn copy_command_output(&mut self, cx: &mut Context<Self>) {
        let output = self
            .command_review
            .as_ref()
            .map(|r| &r.command.output)
            .or_else(|| self.backend.commands().records().last().map(|c| &c.output));
        if let Some(output) = output {
            cx.write_to_clipboard(ClipboardItem::new_string(output.clone()));
        }
    }

    pub fn copy_last_output(&mut self, cx: &mut Context<Self>) {
        if let Some(command) = self.backend.commands().records().last() {
            cx.write_to_clipboard(ClipboardItem::new_string(command.output.clone()));
        }
    }

    pub fn return_to_live(&mut self, cx: &mut Context<Self>) {
        self.command_review = None;
        self.search = None;
        self.retained_output = None;
        self.backend.set_selection(None);
        cx.notify();
    }

    fn send_key(&mut self, action: &SendKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.searching_input(window, cx) || !self.marked.is_empty() {
            cx.propagate();
            return;
        }
        if action.key == "escape" && self.search.is_some() {
            return self.close_search(&CloseSearch, window, cx);
        }
        if let Ok(key) = Keystroke::parse(&action.key) {
            if let Some(bytes) = key_bytes(&key, self.backend.screen().application_cursor()) {
                self.write(bytes, cx);
            }
        }
    }

    pub(super) fn can_sync_folder(&self) -> bool {
        self.marked.is_empty() && self.backend.can_sync_folder()
    }

    pub(super) fn sync_folder(&mut self, directory: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        if !self.marked.is_empty() {
            return Err("Finish or cancel terminal text composition before syncing its folder".into());
        }
        self.backend.sync_folder(directory)?;
        cx.emit(MetadataChanged);
        self.return_to_live(cx);
        cx.notify();
        Ok(())
    }

    pub fn send_selection(&mut self, text: &str, execute: bool, cx: &mut Context<Self>) -> Result<(), String> {
        let bytes = orlo_terminal::input::selection_bytes(text, self.backend.screen().bracketed_paste(), execute)?;
        self.write(bytes, cx);
        self.backend.error().map_or(Ok(()), |error| Err(error.to_string()))
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.searching_input(window, cx) || !self.marked.is_empty() {
            return;
        }
        if let Some(bytes) = key_bytes(&event.keystroke, self.backend.screen().application_cursor()) {
            self.write(bytes, cx);
            cx.stop_propagation();
        }
    }

    fn copy(&mut self, _: &crate::editor::Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .display_selection()
            .map(|(a, b)| {
                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                self.display_screen().contents_between(a.0, a.1, b.0, b.1)
            })
            .or_else(|| self.search.as_ref().and_then(|s| s.results.as_ref())?.selected_text());
        if let Some(text) = text {
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    fn paste(&mut self, _: &crate::editor::Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.search = None;
            self.retained_output = None;
            self.command_review = None;
            self.backend.paste(&text);
            self.wake(cx);
            cx.notify();
        }
    }

    fn select_all(&mut self, _: &crate::editor::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let (rows, cols) = self.display_screen().size();
        self.set_display_selection(Some(((0, 0), (rows - 1, cols))));
        cx.notify();
    }

    fn position(&self, point: Point<Pixels>) -> Option<(u16, u16)> {
        let layout = self.layout?;
        let (rows, cols) = self.backend.screen().size();
        Some((
            ((f32::from(point.y - layout.bounds.top()) / f32::from(layout.line_height)).floor() as i32)
                .clamp(0, rows as i32 - 1) as u16,
            ((f32::from(point.x - layout.bounds.left()) / f32::from(layout.cell_width)).round() as i32)
                .clamp(0, cols as i32) as u16,
        ))
    }

    fn mouse_position(&self, point: Point<Pixels>, clamp: bool) -> Option<(u16, u16)> {
        let layout = self.layout?;
        if !clamp && !layout.bounds.contains(&point) {
            return None;
        }
        let (rows, cols) = self.backend.screen().size();
        Some((
            (f32::from(point.y - layout.bounds.top()) / f32::from(layout.line_height))
                .floor()
                .clamp(0., rows as f32 - 1.) as u16,
            (f32::from(point.x - layout.bounds.left()) / f32::from(layout.cell_width))
                .floor()
                .clamp(0., cols as f32 - 1.) as u16,
        ))
    }

    fn report_mouse(
        &mut self,
        kind: MouseKind,
        position: (u16, u16),
        modifiers: gpui::Modifiers,
        button: Option<MouseButton>,
        cx: &mut Context<Self>,
    ) {
        if let Some(bytes) = self.mouse_report(kind, position, modifiers, button) {
            self.backend.write(bytes);
            self.wake(cx);
            cx.notify();
        }
    }

    /// The bytes reporting a mouse event to the program, in the protocol it asked for.
    fn mouse_report(
        &self,
        kind: MouseKind,
        position: (u16, u16),
        modifiers: gpui::Modifiers,
        button: Option<MouseButton>,
    ) -> Option<Vec<u8>> {
        let button = match button {
            Some(MouseButton::Left) => Some(0),
            Some(MouseButton::Middle) => Some(1),
            Some(MouseButton::Right) => Some(2),
            _ => None,
        };
        let modifiers = orlo_terminal::Modifiers {
            shift: modifiers.shift,
            alt: modifiers.alt,
            control: modifiers.control,
            platform: modifiers.platform,
        };
        mouse::mouse_bytes(
            self.backend.screen(),
            mouse::Mouse { kind, button, row: position.0, col: position.1, modifiers },
        )
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus(window);
        if let Some(p) = self.position(e.position) {
            if e.modifiers.platform && !e.modifiers.shift {
                if let Some(link) = self
                    .mouse_position(e.position, false)
                    .and_then(|cell| orlo_terminal::links::link_at(self.display_screen(), cell))
                {
                    match link {
                        orlo_terminal::links::Link::Url(url) => cx.open_url(&url),
                        orlo_terminal::links::Link::File { path, line, column } => {
                            window.dispatch_action(Box::new(super::OpenTerminalFile { path, line, column }), cx)
                        }
                    }
                    cx.stop_propagation();
                    return;
                }
            }
            if self.search.is_none()
                && self.retained_output.is_none()
                && self.command_review.is_none()
                && !e.modifiers.shift
                && !e.modifiers.platform
                && mouse::requested(self.backend.screen())
            {
                if let Some(position) = self.mouse_position(e.position, false) {
                    self.reporting_button = Some(e.button);
                    self.last_mouse = Some(position);
                    self.report_mouse(MouseKind::Press, position, e.modifiers, Some(e.button), cx);
                }
                cx.stop_propagation();
                return;
            }
            if e.button != MouseButton::Left {
                return;
            }
            self.set_display_selection(Some((p, p)));
            self.dragging = true;
            cx.notify();
        }
    }
    fn mouse_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            self.dragging = false;
            self.reporting_button = None;
            self.last_mouse = None;
            return;
        }
        if self.dragging {
            if let Some(position) = self.position(e.position) {
                if self.extend_display_selection(position) {
                    cx.notify();
                }
            }
        } else if self.search.is_none() && self.retained_output.is_none() && self.command_review.is_none() {
            if let Some(position) = self.mouse_position(e.position, self.reporting_button.is_some()) {
                if self.last_mouse != Some(position) {
                    self.last_mouse = Some(position);
                    self.report_mouse(MouseKind::Motion, position, e.modifiers, self.reporting_button, cx);
                }
            }
        }
    }

    fn mouse_up(&mut self, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.dragging = false;
        if let Some(button) = self.reporting_button.take() {
            if self.focus_handle.is_focused(window) {
                if let Some(position) = self.mouse_position(e.position, true) {
                    let mut modifiers = e.modifiers;
                    modifiers.shift = false;
                    modifiers.platform = false;
                    self.report_mouse(MouseKind::Release, position, modifiers, Some(button), cx);
                }
            }
        }
        self.last_mouse = None;
    }
    fn scroll(&mut self, e: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let dy = f32::from(e.delta.pixel_delta(px(15.)).y);
        let lines = scroll_steps(&mut self.scroll_rest, dy / 15.);
        if self.search.is_none()
            && self.retained_output.is_none()
            && self.command_review.is_none()
            && !e.modifiers.shift
            && mouse::requested(self.backend.screen())
            && self.focus_handle.is_focused(window)
        {
            if let Some(position) = self.mouse_position(e.position, false) {
                // a flick's steps go as one input: one each would overflow the terminal's queue
                let kind = if lines > 0 { MouseKind::WheelUp } else { MouseKind::WheelDown };
                let steps =
                    (0..lines.unsigned_abs().min(32)).map(|_| self.mouse_report(kind, position, e.modifiers, None));
                let bytes: Vec<u8> = steps.flatten().flatten().collect();
                if !bytes.is_empty() {
                    self.backend.write(bytes);
                    self.wake(cx);
                    cx.notify();
                }
                cx.stop_propagation();
                return;
            }
        }
        self.pin_search();
        let scrolled = if let Some(review) = &mut self.command_review {
            review.screen.set_scrollback(review.screen.scrollback().saturating_add_signed(lines));
            true
        } else if let Some(results) = self.output_snapshot_mut() {
            results.scroll_lines(lines);
            self.backend.set_selection(None);
            true
        } else {
            self.backend.scroll_lines(lines)
        };
        if scrolled {
            cx.notify();
            cx.stop_propagation();
        }
    }
}

impl Focusable for Terminal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Terminal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self._blur.is_none() {
            self._blur = Some(cx.on_blur(&self.focus_handle.clone(), window, |t, _, cx| {
                if let Some(button) = t.reporting_button.take() {
                    if let Some(position) = t.last_mouse {
                        t.report_mouse(MouseKind::Release, position, gpui::Modifiers::default(), Some(button), cx);
                    }
                }
                t.dragging = false;
                t.last_mouse = None;
            }));
        }
        let review_description = self.command_review.as_ref().map(|r| r.description.clone());
        let message = self
            .backend
            .error()
            .map(str::to_owned)
            .or_else(|| self.backend.folder_sync_pending().then(|| "Waiting for the terminal folder report…".into()))
            .or_else(|| {
                self.backend
                    .exited()
                    .map(|code| format!("Shell exited ({code}). Close Terminal, then open it to start another."))
            });
        let search = self.render_search(message.clone(), cx);
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .bg(c(theme::EDITOR_BG))
            .text_color(c(theme::EDITOR_FG))
            .font_family(theme::EDITOR_FONT)
            .text_size(px(theme::EDITOR_FONT_SIZE))
            .line_height(px(15.))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::open_search))
            .on_action(cx.listener(Self::close_search))
            .on_action(cx.listener(Self::next_match))
            .on_action(cx.listener(Self::previous_match))
            .on_action(cx.listener(Self::toggle_search_case))
            .on_action(cx.listener(Self::toggle_search_regex))
            .on_action(cx.listener(Self::resume_search))
            .on_action(cx.listener(Self::send_key))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::select_all))
            .on_key_down(cx.listener(Self::on_key_down))
            // Find, command review and retained output share one constant-height toolbar.
            .when_some(search, |d, search| d.child(search))
            .when(self.search.is_none() && self.retained_output.is_none(), |d| {
                d.child(
                    crate::chrome::ui::panel_header()
                        .font_family(theme::UI_FONT)
                        .text_size(px(11.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(message.clone().or(review_description).unwrap_or_else(|| "Live output".into())),
                        )
                        .when(!self.backend.commands().records().is_empty(), |d| {
                            d.child(crate::chrome::ui::ghost_button("previous-term-command", "Previous").on_click(
                                cx.listener(|t, _: &gpui::ClickEvent, w, cx| {
                                    t.focus(w);
                                    t.review_command(false, cx);
                                }),
                            ))
                            .child(crate::chrome::ui::ghost_button("next-term-command", "Next").on_click(cx.listener(
                                |t, _: &gpui::ClickEvent, w, cx| {
                                    t.focus(w);
                                    t.review_command(true, cx);
                                },
                            )))
                            .child(
                                crate::chrome::ui::ghost_button("copy-term-output", "Copy Output")
                                    .on_click(cx.listener(|t, _: &gpui::ClickEvent, _, cx| t.copy_command_output(cx))),
                            )
                        })
                        .child(crate::chrome::ui::ghost_button("term-live", "Live Output").on_click(cx.listener(
                            |t, _: &gpui::ClickEvent, w, cx| {
                                t.return_to_live(cx);
                                t.focus(w);
                            },
                        ))),
                )
            })
            .when(self.retained_output.is_some(), |d| {
                d.child(
                    crate::chrome::ui::panel_header()
                        .font_family(theme::UI_FONT)
                        .text_size(px(11.))
                        .child(
                            div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(
                                message
                                    .clone()
                                    .map_or_else(|| "Retained output".into(), |m| format!("Retained output · {m}")),
                            ),
                        )
                        .child(
                            crate::chrome::ui::ghost_button("term-search-live", "Live Output")
                                .debug_selector(|| "term-search-live".into())
                                .on_click(cx.listener(|t, _: &gpui::ClickEvent, w, cx| {
                                    t.return_to_live(cx);
                                    t.focus(w);
                                })),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::mouse_up))
                    .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(MouseButton::Right, cx.listener(Self::mouse_up))
                    .on_scroll_wheel(cx.listener(Self::scroll))
                    .child(TerminalElement(cx.entity())),
            )
    }
}

fn utf16_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        if units >= offset {
            return byte;
        }
        units += c.len_utf16();
    }
    text.len()
}

impl EntityInputHandler for Terminal {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let a = utf16_byte(&self.marked, range.start);
        let b = utf16_byte(&self.marked, range.end).max(a);
        *actual = Some(self.marked[..a].encode_utf16().count()..self.marked[..b].encode_utf16().count());
        Some(self.marked[a..b].to_string())
    }
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.marked_selection.clone(), reversed: false })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| 0..self.marked.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let text = std::mem::take(&mut self.marked);
        self.marked_selection = 0..0;
        if !text.is_empty() {
            self.write(text.into_bytes(), cx);
        }
    }
    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        self.marked_selection = 0..0;
        self.write(text.as_bytes().to_vec(), cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.unwrap_or(0..self.marked.encode_utf16().count());
        let a = utf16_byte(&self.marked, range.start);
        let b = utf16_byte(&self.marked, range.end).max(a);
        self.marked.replace_range(a..b, text);
        let start = self.marked[..a].encode_utf16().count();
        let n = text.encode_utf16().count();
        let selected = selected.unwrap_or(n..n);
        self.marked_selection = start + selected.start.min(n)..start + selected.end.min(n);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout?;
        let (row, col) = self.backend.screen().cursor_position();
        Some(Bounds::new(
            layout.bounds.origin + point(layout.cell_width * col as f32, layout.line_height * row as f32),
            size(layout.cell_width, layout.line_height),
        ))
    }
    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.marked_selection.end)
    }
}

struct TerminalElement(Entity<Terminal>);
struct TerminalPaint {
    glyphs: Vec<(Point<Pixels>, ShapedLine)>,
    backgrounds: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    line_height: Pixels,
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = TerminalPaint;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> TerminalPaint {
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = px((f32::from(font_size) * 1.25).round());
        let run = TextRun {
            len: 1,
            font: font.clone(),
            color: c(theme::EDITOR_FG),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let cell_width =
            window.text_system().shape_line(" ".into(), font_size, std::slice::from_ref(&run), None).width.max(px(1.));
        self.0.update(cx, |terminal, cx| {
            let rows = (f32::from(bounds.size.height) / f32::from(line_height)).floor().clamp(1., 256.) as u16;
            let cols = (f32::from(bounds.size.width) / f32::from(cell_width)).floor().clamp(2., 512.) as u16;
            if terminal.backend.screen().size() != (rows, cols) {
                terminal.backend.resize(rows, cols);
                terminal.retained_output = None;
                if let Some(review) = &mut terminal.command_review {
                    review.screen = review.command.review_screen(rows, cols);
                }
                terminal.refresh_search(true, cx);
            }
            terminal.layout = Some(TerminalLayout { bounds, cell_width, line_height });
            let screen = terminal.display_screen();
            let mut p = TerminalPaint { glyphs: Vec::new(), backgrounds: Vec::new(), cursor: None, line_height };
            let selection = terminal.display_selection().map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
            for row in 0..rows {
                for col in 0..cols {
                    let Some(cell) = screen.cell(row, col) else {
                        continue;
                    };
                    let origin = bounds.origin + point(cell_width * col as f32, line_height * row as f32);
                    let mut fg = terminal_color(cell.fgcolor(), true);
                    let mut bg = terminal_color(cell.bgcolor(), false);
                    if cell.inverse() {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    if cell.dim() {
                        fg.a *= 0.6;
                    }
                    if terminal.search.as_ref().and_then(|s| s.results.as_ref()).is_some_and(|r| r.highlights(row, col))
                    {
                        bg = c(theme::SELECTION);
                    }
                    if selection.is_some_and(|(a, b)| (row, col) >= a && (row, col) < b) {
                        bg = c(theme::SELECTION);
                    }
                    if bg != c(theme::EDITOR_BG) {
                        p.backgrounds.push(fill(Bounds::new(origin, size(cell_width, line_height)), bg));
                    }
                    if !cell.has_contents() || cell.is_wide_continuation() {
                        continue;
                    }
                    let mut run = run.clone();
                    run.len = cell.contents().len();
                    run.color = fg;
                    if cell.bold() {
                        run.font.weight = FontWeight::BOLD;
                    }
                    if cell.italic() {
                        run.font.style = FontStyle::Italic;
                    }
                    if cell.underline() {
                        run.underline = Some(underline(fg));
                    }
                    p.glyphs.push((
                        origin,
                        window.text_system().shape_line(cell.contents().to_string().into(), font_size, &[run], None),
                    ));
                }
            }
            let (row, col) = screen.cursor_position();
            let caret =
                bounds.origin + point(cell_width * col.min(cols - 1) as f32, line_height * row.min(rows - 1) as f32);
            if terminal.search.is_none()
                && terminal.retained_output.is_none()
                && !screen.hide_cursor()
                && screen.scrollback() == 0
                && terminal.focus_handle.is_focused(window)
            {
                p.cursor = Some(fill(Bounds::new(caret, size(px(1.5), line_height)), c(theme::CARET)));
            }
            if !terminal.marked.is_empty() {
                let mut run = run.clone();
                run.len = terminal.marked.len();
                run.background_color = Some(c(theme::EDITOR_BG));
                run.underline = Some(underline(run.color));
                p.glyphs.push((
                    caret,
                    window.text_system().shape_line(terminal.marked.clone().into(), font_size, &[run], None),
                ));
            }
            p
        })
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        p: &mut TerminalPaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let handle = self.0.read(cx).focus_handle.clone();
        window.handle_input(&handle, ElementInputHandler::new(bounds, self.0.clone()), cx);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in p.backgrounds.drain(..) {
                window.paint_quad(quad);
            }
            for (origin, glyph) in &p.glyphs {
                glyph.paint(*origin, p.line_height, window, cx).ok();
            }
            if let Some(cursor) = p.cursor.take() {
                window.paint_quad(cursor);
            }
        });
    }
}

fn underline(color: Hsla) -> UnderlineStyle {
    UnderlineStyle { thickness: px(1.), color: Some(color), wavy: false }
}

fn terminal_color(color: Color, foreground: bool) -> Hsla {
    match color {
        Color::Default => c(if foreground { theme::EDITOR_FG } else { theme::EDITOR_BG }),
        Color::Rgb(r, g, b) => gpui::rgb((r as u32) << 16 | (g as u32) << 8 | b as u32).into(),
        Color::Idx(i) => {
            let rgb = if i < 16 {
                [
                    0x20242a, 0xec5f66, 0x99c794, 0xfac863, 0x6699cc, 0xc594c5, 0x5fb3b3, 0xd8dee9, 0x65737e, 0xf99157,
                    0xabd3a6, 0xffdb85, 0x85b3df, 0xd8a8d8, 0x79cdcd, 0xffffff,
                ][i as usize]
            } else if i < 232 {
                let n = i as u32 - 16;
                let component = |v| if v == 0 { 0 } else { 55 + v * 40 };
                component(n / 36) << 16 | component(n / 6 % 6) << 8 | component(n % 6)
            } else {
                let v = 8 + (i as u32 - 232) * 10;
                v << 16 | v << 8 | v
            };
            gpui::rgb(rgb).into()
        }
    }
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod link_tests;

#[cfg(test)]
#[path = "mouse_tests.rs"]
mod mouse_tests;

#[cfg(test)]
#[path = "session_tests.rs"]
mod session_tests;

#[cfg(test)]
#[path = "send_selection_tests.rs"]
mod send_selection_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use orlo_terminal::{PtyEvent, PtySession};
    use std::time::Instant;

    #[test]
    fn command_boundary_actions_include_copy_last_output() {
        let labels: Vec<_> = crate::chrome::quick_panel::command_items().into_iter().map(|item| item.label).collect();
        for label in ["Terminal: Previous Command", "Terminal: Next Command", "Terminal: Copy Last Output"] {
            assert!(labels.iter().any(|name| name == label), "missing command: {label}");
        }
    }

    #[gpui::test]
    fn integration_preferences_only_affect_new_profile_launches_and_close_cleans_owned_files(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::config::settings::Settings;
        use crate::workspace::terminal::{CloseTerminalTab, SelectTerminalTab};
        use crate::workspace::ui_tests::{settle_until, temp, window};
        use crate::workspace::{NewTerminal, ShowTerminal};
        use std::os::unix::fs::PermissionsExt;
        let directory = temp("t06-integration-launch");
        let executable = directory.join("zsh");
        std::fs::write(&executable, "#!/bin/sh\nstty -echo\nprintf 'INTEGRATION:%s\\nARGV:%s:%s\\n' \"${ORLO_INTEGRATION_DIR:-none}\" \"$1\" \"$2\"\nprintf 'PID:%s:END\\n' \"$$\"\nIFS= read -r value\nprintf 'RECEIVED:%s\\n' \"$value\"\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (ws, cx) = window(cx);
        let mut terminals = Vec::new();
        for enabled in [false, true, false] {
            let terminal = ws.update_in(cx, |ws, window, cx| {
                let (settings, warning) = crate::config::settings::parse(
                    &serde_json::json!({"terminal":{
                        "shell_integration":enabled,"default_profile":"Fixture","profiles":{"Fixture":{
                            "shell":executable,"arguments":["-l","-i"],"directory":directory,"scrollback":3
                        }}
                    }})
                    .to_string(),
                )
                .unwrap();
                assert!(warning.is_none(), "{warning:?}");
                cx.global_mut::<Settings>().terminal = settings.terminal;
                if ws.terminal.is_none() {
                    ws.show_terminal(&ShowTerminal, window, cx);
                } else {
                    ws.new_terminal(&NewTerminal, window, cx);
                }
                let terminal = ws.terminal.clone().unwrap();
                terminal.update(cx, |t, _| t._poll = None);
                terminal
            });
            settle_until(cx, |cx| {
                terminal.update(cx, |t, cx| {
                    t.drain(cx);
                    let output = t.backend.screen().contents();
                    output.contains("ARGV:-l:-i") && output.contains(":END")
                })
            });
            let output = terminal.read_with(cx, |t, _| {
                assert!(t.backend.pty().is_some());
                assert_eq!(t.backend.screen().scrollback_limit(), 3);
                t.backend.screen().contents()
            });
            assert_eq!(output.contains("INTEGRATION:none"), !enabled, "{output}");
            terminals.push(terminal);
        }
        let integrated = &terminals[1];
        let (path, pid) = integrated.read_with(cx, |t, _| {
            let text = t.backend.screen().contents();
            let pid = text
                .lines()
                .find_map(|line| line.strip_prefix("PID:")?.strip_suffix(":END")?.parse::<libc::pid_t>().ok())
                .expect("fixture's complete process id");
            (
                PathBuf::from(
                    text.split("ARGV:").next().unwrap().strip_prefix("INTEGRATION:").unwrap().replace('\n', "").trim(),
                ),
                pid,
            )
        });
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "fixture is alive before the close");
        assert!(path.join(".zshrc").is_file());
        for terminal in &terminals {
            assert!(terminal.read_with(cx, |t, _| t.backend.pty().is_some()), "settings must not replace old PTYs");
        }
        ws.update_in(cx, |ws, window, cx| {
            ws.select_terminal_tab(&SelectTerminalTab { id: terminals[0].entity_id() }, window, cx);
            ws.close_terminal_tab(&CloseTerminalTab { id: integrated.entity_id() }, window, cx);
            assert_eq!(ws.terminal.as_ref(), Some(&terminals[0]));
        });
        settle_until(cx, |_| {
            let reaped = unsafe { libc::kill(pid, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            reaped && !path.exists()
        });
        assert!(!path.exists(), "admitted close releases private hooks even while entity is retained");
        assert!(integrated.read_with(cx, |t, _| t.backend.pty().is_none()));
        for terminal in [&terminals[0], &terminals[2]] {
            terminal.update(cx, |t, cx| t.write(b"still-live\r".to_vec(), cx));
            settle_until(cx, |cx| {
                terminal.update(cx, |t, cx| {
                    t.drain(cx);
                    t.backend.screen().contents().contains("RECEIVED:still-live")
                })
            });
        }
    }

    #[gpui::test]
    fn command_review_navigates_copies_output_and_returns_to_the_live_pty(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::settle_until;
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            let mut t = Terminal::new(std::env::temp_dir(), cx);
            t._poll = None;
            t.backend = TerminalSession::from_command(command(
                "printf '\\033]133;B\\007one\\r\\n\\033]133;C\\007first\\r\\n\\033]133;D;0\\007\\033]133;B\\007two\\r\\n\\033]133;C\\007second\\r\\n\\033]133;D;2\\007'; read value"
            ), Default::default());
            t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.commands().records().len() == 2
            })
        });
        terminal.update(cx, |t, cx| {
            t.review_command(false, cx);
            assert!(t.command_review.as_ref().unwrap().description.contains("exit 2"));
            assert_eq!(t.display_screen().contents(), "second");
            t.copy_command_output(cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("second"));
            t.review_command(false, cx);
            assert_eq!(t.display_screen().contents(), "first");
            t.review_command(true, cx);
            assert_eq!(t.display_screen().contents(), "second");
            t.review_command(true, cx);
            assert!(t.command_review.is_none());
            assert!(t.backend.exited().is_none());
        });
    }

    #[gpui::test]
    fn last_output_and_review_actions_exclude_prompts_and_never_write_to_the_pty(cx: &mut gpui::TestAppContext) {
        use crate::config::settings::Settings;
        use crate::workspace::ui_tests::{settle_until, temp, window};
        use crate::workspace::{
            CopyLastTerminalOutput, CopyTerminalCommandOutput, NextTerminalCommand, PreviousTerminalCommand,
            ShowTerminal,
        };
        let marker = temp("t06-output-never-executed").join("executed");
        let payload = format!("printf SHOULD_NOT_RUN > '{}'", marker.display());
        let (ws, cx) = window(cx);
        let terminal = ws.update_in(cx, |ws, window, cx| {
            assert!(cx.build_action("terminal::CopyLastTerminalOutput", None).is_ok());
            let settings = &mut cx.global_mut::<Settings>().terminal;
            settings.shell = Some("/bin/sh".into());
            settings.arguments = Some(vec!["-c".into(), r#"stty raw -echo
printf '\033]133;A\007PROMPT> \033]133;B\007first command\r\n\033]133;C\007older output\r\n\033]133;D;0\007\033]133;A\007PROMPT> \033]133;B\007second command\r\n\033]133;C\007'
printf '%s\r\n' "$1"
printf '\033]133;D;2\007\033]133;A\007PROMPT> \033]133;B\007READY\r\nBYTE:'
dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'
printf '\r\nDONE\r\n'
exec sleep 60"#.into(), "orlo-t06".into(), payload.clone()]);
            ws.show_terminal(&ShowTerminal, window, cx);
            let terminal = ws.terminal.clone().unwrap();
            terminal.update(cx, |t, _| t._poll = None);
            terminal
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.commands().records().len() == 2 && t.backend.screen().contents().contains("READY")
            })
        });
        ws.update_in(cx, |ws, window, cx| {
            ws.previous_terminal_command(&PreviousTerminalCommand, window, cx);
            assert_eq!(terminal.read(cx).command_review.as_ref().unwrap().command.output, payload);
            ws.previous_terminal_command(&PreviousTerminalCommand, window, cx);
            assert_eq!(terminal.read(cx).display_screen().contents(), "older output");
            ws.copy_terminal_command_output(&CopyTerminalCommandOutput, window, cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("older output"));
            ws.copy_last_terminal_output(&CopyLastTerminalOutput, window, cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some(payload.as_str()));
            assert_eq!(terminal.read(cx).display_screen().contents(), "older output", "copy does not navigate");
            ws.next_terminal_command(&NextTerminalCommand, window, cx);
            ws.next_terminal_command(&NextTerminalCommand, window, cx);
            assert!(terminal.read(cx).command_review.is_none());
            assert!(terminal.read(cx).backend.exited().is_none());
            terminal.update(cx, |t, cx| t.write(b"Q".to_vec(), cx));
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("DONE")
            })
        });
        terminal.read_with(cx, |t, _| {
            assert!(
                t.backend.screen().contents().contains("BYTE:51"),
                "navigation/copy must leave the first input byte untouched: {}",
                t.backend.screen().contents()
            )
        });
        assert!(!marker.exists(), "copied output must never be executed");
    }

    #[gpui::test]
    fn unmarked_terminal_copy_and_navigation_keep_plain_input_output_usable(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::settle_until;
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            let mut t = Terminal::new(std::env::temp_dir(), cx);
            t._poll = None;
            t.backend = TerminalSession::from_command(command("stty raw -echo; printf 'plain prompt> error\\r\\nREADY\\r\\nBYTE:'; dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' \\n'; printf '\\r\\nDONE\\r\\n'"), Default::default());
            t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        terminal.update(cx, |t, cx| {
            assert!(t.backend.commands().records().is_empty());
            let before = t.display_screen().contents();
            cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into()));
            t.review_command(false, cx);
            t.review_command(true, cx);
            t.copy_command_output(cx);
            t.copy_last_output(cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("unchanged"));
            assert_eq!(t.display_screen().contents(), before);
            assert!(t.command_review.is_none());
            t.write(b"Q".to_vec(), cx);
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("DONE")
            })
        });
        terminal.read_with(cx, |t, _| assert!(t.backend.screen().contents().contains("BYTE:51")));
    }

    #[gpui::test]
    fn displayed_command_copy_stays_pinned_when_live_output_evicts_its_record(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::settle_until;
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            let mut t = Terminal::new(std::env::temp_dir(), cx);
            t._poll = None;
            t.backend = TerminalSession::from_command(
                command(
                    r#"stty -echo
printf '\033]133;C\007pinned output\033]133;D;0\007\r\nREADY1\r\n'
IFS= read -r value
i=0; while [ "$i" -lt 40 ]; do printf '\033]133;C\007new%s\033]133;D;0\007' "$i"; i=$((i+1)); done
printf '\r\nREADY2\r\n'; IFS= read -r value"#,
                ),
                Default::default(),
            );
            t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY1")
            })
        });
        terminal.update(cx, |t, cx| {
            t.review_command(false, cx);
            assert_eq!(t.command_review.as_ref().unwrap().id, 1);
            t.backend.write(b"advance\r".to_vec());
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY2")
            })
        });
        terminal.update(cx, |t, cx| {
            assert_eq!(t.backend.commands().records().len(), 32);
            assert!(t.backend.commands().records()[0].id > 1);
            assert_eq!(t.display_screen().contents(), "pinned output");
            t.copy_command_output(cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("pinned output"));
            t.copy_last_output(cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("new39"));
            t.review_command(false, cx);
            assert_eq!(t.display_screen().contents(), "new39", "an evicted navigation anchor resolves to newest");
        });
    }

    #[gpui::test]
    fn terminal_selection_refuses_pending_shared_edits(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::window;
        use crate::workspace::{ExecuteSelectionInTerminal, NewViewIntoFile, SendSelectionToTerminal};
        for execute in [false, true] {
            let (ws, cx) = window(cx);
            let first = ws.update_in(cx, |ws, w, cx| {
                let first = ws.active_editor().unwrap();
                first.update(cx, |e, cx| e.set_text("0123456789", cx));
                ws.new_view_into_file(&NewViewIntoFile, w, cx);
                ws.active_editor().unwrap().update(cx, |e, cx| e.select_range(5..10, cx));
                first
            });
            ws.update_in(cx, |ws, w, cx| {
                first.update(cx, |e, cx| {
                    e.edit(cx, |s| {
                        s.select_all();
                        s.backspace();
                    })
                });
                let editor = ws.active_editor().unwrap();
                let state = editor.read(cx).read_state(cx);
                assert_ne!(state.view.revision(), state.doc.version);
                if execute {
                    ws.execute_selection_in_terminal(&ExecuteSelectionInTerminal, w, cx);
                } else {
                    ws.send_selection_to_terminal(&SendSelectionToTerminal, w, cx);
                }
                assert!(ws.terminal.is_none(), "unmapped selections must not create a terminal");
                assert!(ws.status.as_ref().unwrap().0.contains("updating"));
                assert_eq!(editor.read(cx).text(cx), "");
            });
            cx.run_until_parked();
        }
    }

    #[gpui::test]
    fn terminal_selection_refuses_shared_ime_without_committing(cx: &mut gpui::TestAppContext) {
        use crate::config::settings::{Settings, TerminalSettings};
        use crate::workspace::ui_tests::{temp, two_windows};
        use crate::workspace::{ExecuteSelectionInTerminal, SendSelectionToTerminal};
        for execute in [false, true] {
            let path = temp(&format!("terminal-source-composition-{execute}")).join("source.txt");
            std::fs::write(&path, "original").unwrap();
            let (first, second, mut a, mut b) = two_windows(cx);
            first.update_in(&mut a, |ws, w, cx| ws.open_path(path.clone(), w, cx));
            second.update_in(&mut b, |ws, w, cx| ws.open_path(path.clone(), w, cx));
            let composing = first.read_with(&a, |ws, _| ws.active_editor().unwrap());
            composing.update_in(&mut a, |e, w, cx| {
                e.perform_action(&crate::editor::ToggleOvertype, w, cx);
                e.sel(cx).set_cursor(0);
                e.replace_and_mark_text_in_range(None, "界", None, w, cx);
            });
            b.run_until_parked();
            let before = composing.read_with(&a, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version));
            second.update_in(&mut b, |ws, w, cx| {
                cx.global_mut::<Settings>().terminal = TerminalSettings {
                    shell: Some("/bin/sh".into()),
                    arguments: Some(vec!["-c".into(), "cat".into()]),
                    ..Default::default()
                };
                let editor = ws.active_editor().unwrap();
                editor.update(cx, |e, cx| e.select_range(0..1, cx));
                let state = editor.read(cx).read_state(cx);
                assert_eq!(state.view.revision(), state.doc.version, "test composition independently of revision lag");
                if execute {
                    ws.execute_selection_in_terminal(&ExecuteSelectionInTerminal, w, cx);
                } else {
                    ws.send_selection_to_terminal(&SendSelectionToTerminal, w, cx);
                }
                assert!(ws.terminal.is_none(), "provisional text must not create a terminal");
                assert!(ws.status.as_ref().unwrap().0.contains("composition"));
            });
            assert_eq!(before, composing.read_with(&a, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)));
        }
    }

    #[gpui::test]
    fn sending_read_only_editor_text_reuses_the_pty_and_only_execute_adds_enter(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::{settle_until, window};
        use crate::workspace::{ExecuteSelectionInTerminal, SendSelectionToTerminal};
        let (ws, cx) = window(cx);
        let terminal = ws.update_in(cx, |ws, w, cx| {
            ws.show_terminal(&crate::workspace::ShowTerminal, w, cx);
            let terminal = ws.terminal.clone().unwrap();
            terminal.update(cx, |t, _| {
                t._poll = None;
                t.backend = TerminalSession::from_command(
                    command("stty -echo; echo READY; IFS= read -r value; printf 'received:%s\\n' \"$value\""),
                    Default::default(),
                );
            });
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, cx| {
                e.set_text("中😀", cx);
                e.select_range(0..2, cx);
                e.document.update(cx, |d, _| d.model.read_only = true);
            });
            terminal
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        ws.update_in(cx, |ws, w, cx| {
            ws.send_selection_to_terminal(&SendSelectionToTerminal, w, cx);
            ws.execute_selection_in_terminal(&ExecuteSelectionInTerminal, w, cx);
            assert_eq!(ws.terminal.as_ref(), Some(&terminal));
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).text(cx), "中😀");
            assert!(editor.read(cx).doc(cx).read_only);
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("received:中😀中😀")
            })
        });
    }

    #[gpui::test]
    fn preferences_launch_the_selected_shell_with_literal_arguments(cx: &mut gpui::TestAppContext) {
        use crate::config::settings::{Settings, TerminalSettings};
        use crate::workspace::ui_tests::settle_until;
        cx.update(|cx| {
            cx.set_global(Settings {
                terminal: TerminalSettings {
                    shell: Some("/bin/sh".into()),
                    arguments: Some(vec![
                        "-c".into(),
                        "printf '%s\\n' \"$1\"".into(),
                        "orlo".into(),
                        "$HOME is literal".into(),
                    ]),
                    scrollback: 0,
                    ..Default::default()
                },
                ..Default::default()
            })
        });
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            let mut terminal = Terminal::new(std::env::temp_dir(), cx);
            terminal._poll = None;
            terminal
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("$HOME is literal")
            })
        });
        terminal.update(cx, |t, _| {
            t.backend.scroll_lines(1000);
            assert_eq!(t.backend.screen().scrollback(), 0);
        });
    }

    #[gpui::test]
    fn mouse_reporting_uses_floor_cells_and_shift_drag_selects(cx: &mut gpui::TestAppContext) {
        use crate::workspace::ui_tests::settle_until;
        let (terminal, cx) = cx.add_window_view(|window, cx| {
            let mut t = Terminal::new(std::env::temp_dir(), cx);
            t._poll = None;
            t.backend = TerminalSession::from_command(command(
                "stty raw -echo; printf '\\033[?1000h\\033[?1006hREADY'; dd bs=1 count=9 2>/dev/null | od -An -tx1; printf 'DONE'"
            ), orlo_terminal::TerminalConfiguration::default());
            t.focus(window);
            t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        cx.run_until_parked();
        let (a, b) = terminal.read_with(cx, |t, _| {
            let l = t.layout.unwrap();
            (
                l.bounds.origin + point(l.cell_width * 2.7, l.line_height * 1.8),
                l.bounds.origin + point(l.cell_width * 7., l.line_height * 1.8),
            )
        });
        cx.simulate_mouse_down(a, MouseButton::Left, gpui::Modifiers { shift: true, ..Default::default() });
        cx.simulate_mouse_move(b, Some(MouseButton::Left), gpui::Modifiers::default());
        terminal.read_with(cx, |t, _| {
            assert!(t.backend.selection().is_some_and(|(a, b)| a != b));
            assert!(t.reporting_button.is_none());
        });
        cx.simulate_mouse_up(b, MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_down(a, MouseButton::Left, gpui::Modifiers::default());
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("DONE")
            })
        });
        terminal.read_with(cx, |t, _| {
            assert!(t
                .backend
                .screen()
                .contents()
                .split_whitespace()
                .collect::<Vec<_>>()
                .windows(9)
                .any(|w| w == ["1b", "5b", "3c", "30", "3b", "33", "3b", "32", "4d"]))
        });
    }

    fn command(script: &str) -> orlo_terminal::CommandBuilder {
        let mut cmd = orlo_terminal::CommandBuilder::new("/bin/sh");
        cmd.args(["-c", script]);
        cmd.cwd(std::env::temp_dir());
        cmd
    }

    fn until(session: &PtySession, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = String::new();
        while Instant::now() < deadline {
            if let Ok(event) = session.recv_timeout(Duration::from_millis(50)) {
                match event {
                    PtyEvent::Output(bytes) => output.push_str(&String::from_utf8_lossy(&bytes)),
                    PtyEvent::Error(error) => panic!("PTY error: {error}"),
                    PtyEvent::Exited(_) => break,
                }
                if output.find(needle).is_some_and(|at| output[at..].contains('\n')) {
                    return output;
                }
            }
        }
        panic!("missing {needle:?} in {output:?}");
    }

    #[test]
    fn keys_use_terminal_control_and_application_cursor_sequences() {
        for (key, application, expected) in [
            ("ctrl-c", false, "\x03"),
            ("ctrl-d", false, "\x04"),
            ("ctrl-space", false, "\0"),
            ("up", false, "\x1b[A"),
            ("up", true, "\x1bOA"),
            ("ctrl-left", false, "\x1b[1;5D"),
            ("shift-tab", false, "\x1b[Z"),
            ("delete", false, "\x1b[3~"),
            ("f5", false, "\x1b[15~"),
        ] {
            assert_eq!(
                key_bytes(&Keystroke::parse(key).unwrap(), application),
                Some(expected.as_bytes().to_vec()),
                "{key}"
            );
        }
        assert_eq!(key_bytes(&Keystroke::parse("a").unwrap(), false), None, "ordinary input belongs to the IME");
        assert_eq!(key_bytes(&Keystroke::parse("cmd-c").unwrap(), false), None);
        assert!(!key_bindings().is_empty());
    }

    #[test]
    fn slow_scrolling_adds_up_to_whole_lines() {
        let mut rest = 0.;
        let steps: Vec<isize> = (0..6).map(|_| scroll_steps(&mut rest, 0.2)).collect();
        assert_eq!(steps, [0, 0, 1, 0, 0, 0]);
        // a wheel's whole lines, either way, as before
        assert_eq!(scroll_steps(&mut rest, -3.), -3);
        assert_eq!(scroll_steps(&mut rest, 1.), 1);
    }

    #[gpui::test]
    fn a_quiet_terminal_is_read_less_until_typed_in(cx: &mut gpui::TestAppContext) {
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            let mut terminal = Terminal::new(std::env::temp_dir(), cx);
            terminal._poll = None;
            terminal.backend = TerminalSession::from_command(
                command("stty -echo; echo ready; IFS= read -r value; echo got; read -r value"),
                orlo_terminal::TerminalConfiguration::default(),
            );
            until(terminal.backend.pty().unwrap(), "ready");
            terminal
        });
        terminal.update(cx, |terminal, cx| {
            for _ in 1..QUIET_DRAINS {
                assert_eq!(terminal.drain(cx), Some(POLL));
            }
            assert_eq!(terminal.drain(cx), Some(QUIET_POLL));
            terminal.write(b"x\n".to_vec(), cx);
            assert_eq!(terminal.drain(cx), Some(POLL));
        });
    }

    #[gpui::test]
    fn focused_terminal_commits_ime_and_enter_to_the_real_pty(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let (terminal, cx) = cx.add_window_view(|window, cx| {
            let mut terminal = Terminal::new(std::env::temp_dir(), cx);
            terminal._poll = None;
            terminal.backend = TerminalSession::from_command(
                command("stty -echo; echo ready; IFS= read -r value; printf 'received:%s\\n' \"$value\""),
                orlo_terminal::TerminalConfiguration::default(),
            );
            until(terminal.backend.pty().unwrap(), "ready");
            terminal.focus(window);
            terminal
        });
        cx.run_until_parked();
        terminal.update_in(cx, |terminal, window, cx| {
            assert!(terminal.focus_handle.is_focused(window));
            assert!(terminal.layout.is_some());
            terminal.replace_and_mark_text_in_range(None, "é", None, window, cx);
            terminal.unmark_text(window, cx);
            terminal.replace_and_mark_text_in_range(None, "k", None, window, cx);
            terminal.replace_and_mark_text_in_range(None, "か😀", Some(3..3), window, cx);
            assert_eq!(terminal.marked_text_range(window, cx), Some(0..3));
            assert_eq!(terminal.selected_text_range(false, window, cx).unwrap().range, 3..3);
            terminal.replace_text_in_range(None, "か😀", window, cx);
            assert!(terminal.marked.is_empty());
        });
        cx.simulate_keystrokes("enter");
        terminal.update(cx, |terminal, _| {
            let output = until(terminal.backend.pty().unwrap(), "received:éか😀");
            assert!(output.contains("received:éか😀"));
        });
    }
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod profile_tests;

#[cfg(test)]
#[path = "sync_folder_tests.rs"]
mod sync_folder_tests;
