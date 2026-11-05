//! Sublime's Find in Files panel: Find, Where and Replace rows; regex, case and whole word
//! toggles for matching, and Show Context, Use Buffer, Use gitignore and Preserve Case.
//! Find streams into Find Results; Replace… previews in the results browser.

use crate::chrome::find_panel::{dispatching, error_line, panel_frame, search_field};
use crate::chrome::icons::Icon;
use crate::chrome::ui as kit;
use crate::editing::text::{EditorState, FindOpts};
use crate::editor::Editor;
use crate::searching::results::SearchFlags;
use crate::workspace::{self, Workspace};
use gpui::{actions, div, prelude::*, px, AnyElement, ClickEvent, Context, Entity, KeyBinding, MouseButton, Window};

actions!(find_in_files, [FocusNext, FocusPrev]);

/// Bound after the editor's bindings so they win inside the panel's inputs.
pub fn key_bindings() -> Vec<KeyBinding> {
    let ctx = Some("FindInFilesPanel > Editor");
    vec![
        KeyBinding::new("enter", workspace::RunFindInFiles, ctx),
        KeyBinding::new("alt-enter", workspace::RunFindInFiles, ctx),
        KeyBinding::new("cmd-alt-enter", workspace::ReplaceInFiles, ctx),
        KeyBinding::new("escape", workspace::ClosePanel, ctx),
        KeyBinding::new("tab", FocusNext, ctx),
        KeyBinding::new("shift-tab", FocusPrev, ctx),
        KeyBinding::new("cmd-alt-r", workspace::ToggleFindRegex, ctx),
        KeyBinding::new("cmd-alt-c", workspace::ToggleFindCase, ctx),
        KeyBinding::new("cmd-alt-w", workspace::ToggleFindWord, ctx),
        KeyBinding::new("cmd-alt-a", workspace::TogglePreserveCase, ctx),
        KeyBinding::new("up", workspace::PreviousFindQuery, ctx),
        KeyBinding::new("down", workspace::NextFindQuery, ctx),
        KeyBinding::new("alt-down", workspace::ShowFieldHistory, ctx),
    ]
}

pub struct FindInFilesPanel {
    pub query: Entity<Editor>,
    pub where_: Entity<Editor>,
    pub replace: Entity<Editor>,
    pub opts: FindOpts,
    pub flags: SearchFlags,
    pub error: Option<String>,
}

/// The buttons' column, so the three fields line up.
const CONTROLS_W: f32 = 132.;

impl FindInFilesPanel {
    pub fn new(query: &str, where_: &str, replace: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let input = |text: &str, multiline: bool, cx: &mut Context<Workspace>| {
            cx.new(|cx| {
                let mut e = Editor::new(EditorState::from_text(text), true, cx);
                e.multiline = multiline;
                e.sel(cx).select_all();
                e
            })
        };
        let query = input(query, true, cx);
        let where_ = input(where_, false, cx);
        let replace = input(replace, true, cx);
        for e in [&query, &replace] {
            Editor::on_text_change(e, cx, |ws, cx| ws.validate_fif(cx)).detach();
        }
        // a field's placeholder shows only while it is empty
        for e in [&query, &where_, &replace] {
            cx.observe(e, |_, _, cx| cx.notify()).detach();
        }
        query.read(cx).focus(window);
        Self { query, where_, replace, opts: FindOpts::default(), flags: SearchFlags::default(), error: None }
    }

    /// The fields in Tab order.
    pub fn fields(&self) -> [&Entity<Editor>; 3] {
        [&self.query, &self.where_, &self.replace]
    }

    pub fn render(&self, cx: &mut Context<Workspace>) -> AnyElement {
        type Flip = fn(&mut FindInFilesPanel);
        let toggle = |id: &'static str, i: Icon, tip: &'static str, on: bool, flip: Flip| {
            kit::icon_toggle(id, i, tip, on)
                .debug_selector(|| id.to_string())
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(move |ws, _: &ClickEvent, _, cx| {
                    if let Some(p) = ws.fif_panel_mut() {
                        flip(p);
                    }
                    ws.validate_fif(cx);
                    cx.notify();
                }))
        };
        // a field says what it is for while it is empty; its own options sit inside, at the right
        let field = |e, placeholder, options| search_field(e, Some(placeholder), options, cx);
        let controls = || {
            div().w(px(CONTROLS_W)).flex_none().h(px(26.)).flex().flex_row().items_center().justify_end().gap(px(2.))
        };
        let row = || div().flex().flex_row().items_start().gap(px(6.));
        let (o, f) = (self.opts, self.flags);
        panel_frame("FindInFilesPanel", "close-fif")
            .child(
                row()
                    .child(field(
                        &self.query,
                        "Find",
                        vec![
                            toggle("fif-case", Icon::CaseSensitive, "Case Sensitive  ⌥⌘C", o.case_sensitive, |p| {
                                p.opts.case_sensitive ^= true
                            }),
                            toggle("fif-word", Icon::WholeWord, "Whole Word  ⌥⌘W", o.whole_word, |p| {
                                p.opts.whole_word ^= true
                            }),
                            toggle("fif-regex", Icon::Regex, "Regular Expression  ⌥⌘R", o.regex, |p| {
                                p.opts.regex ^= true
                            }),
                        ],
                    ))
                    .child(
                        controls()
                            .child(toggle("fif-context", Icon::Pilcrow, "Show Context", f.context, |p| {
                                p.flags.context ^= true
                            }))
                            .child(toggle("fif-buffer", Icon::Layers, "Use Buffer", f.use_buffer, |p| {
                                p.flags.use_buffer ^= true
                            }))
                            .child(toggle("fif-gitignore", Icon::EyeOff, "Use .gitignore", f.gitignore, |p| {
                                p.flags.gitignore ^= true
                            }))
                            .child(dispatching(kit::button("fif-run", "Find").ml(px(4.)), workspace::RunFindInFiles)),
                    ),
            )
            .child(
                row()
                    .child(field(
                        &self.where_,
                        "Where: open files and folders",
                        vec![dispatching(
                            kit::icon_button("fif-where-choose", Icon::Ellipsis, "Choose Where…"),
                            workspace::ChooseWhere,
                        )],
                    ))
                    .child(controls()),
            )
            .child(
                row()
                    .child(field(
                        &self.replace,
                        "Replace",
                        vec![toggle(
                            "fif-preserve-case",
                            Icon::PreserveCase,
                            "Preserve Case  ⌥⌘A",
                            f.preserve_case,
                            |p| p.flags.preserve_case ^= true,
                        )],
                    ))
                    .child(
                        controls()
                            .child(dispatching(kit::button("fif-replace", "Replace…"), workspace::ReplaceInFiles)),
                    ),
            )
            .when_some(self.error.clone(), |d, error| d.child(error_line("fif-error", error)))
            .into_any_element()
    }
}
