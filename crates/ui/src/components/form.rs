//! The bottom form panel: Sublime's input panel and its multi-field relatives. A command opens
//! one with `show_input` or `show_form`; Enter hands the values to it, and it answers `Ok` (the
//! panel closes) or `Err` (the panel stays, saying why).

use crate::workspace::*;

actions!(form_panel, [Submit, NextField, PrevField]);

/// Bound after the editor's bindings so they win inside the panel's inputs.
/// Only a text field takes keys, so a form of toggles alone cannot be submitted
/// from the keyboard; upgrade when a command needs one (give the panel a focus handle).
pub fn key_bindings() -> Vec<KeyBinding> {
    let ctx = Some("FormPanel > Editor");
    vec![
        KeyBinding::new("enter", Submit, Some("FormPanel > Editor && !multiline")),
        KeyBinding::new("cmd-enter", Submit, ctx),
        KeyBinding::new("enter", crate::editor::FieldNewline, Some("FormPanel > Editor && multiline")),
        KeyBinding::new("escape", ClosePanel, ctx),
        KeyBinding::new("tab", NextField, ctx),
        KeyBinding::new("shift-tab", PrevField, ctx),
    ]
}

pub enum FieldKind {
    Text,
    /// Enter inserts a newline; Command-Enter submits.
    Multiline,
    /// Starts on or off.
    Toggle(bool),
    /// The options; a click moves to the next, after the last the first.
    Choice(Vec<&'static str>),
}

pub struct Field {
    /// What the command asks `FormValues` for.
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    /// A Text field's starting text, or the Choice option selected first (else the first).
    pub initial: String,
}

impl Field {
    fn new(key: &'static str, label: &'static str, kind: FieldKind, initial: String) -> Self {
        Self { key, label, kind, initial }
    }

    pub(crate) fn text(key: &'static str, label: &'static str, initial: impl Into<String>) -> Self {
        Self::new(key, label, FieldKind::Text, initial.into())
    }

    pub(crate) fn multiline(key: &'static str, label: &'static str, initial: impl Into<String>) -> Self {
        Self::new(key, label, FieldKind::Multiline, initial.into())
    }

    pub(crate) fn toggle(key: &'static str, label: &'static str, on: bool) -> Self {
        Self::new(key, label, FieldKind::Toggle(on), String::new())
    }

    pub(crate) fn choice(key: &'static str, label: &'static str, options: Vec<&'static str>) -> Self {
        Self::new(key, label, FieldKind::Choice(options), String::new())
    }
}

enum Value {
    Text(String),
    Flag(bool),
    Choice(usize),
}

/// What the fields hold when Enter is pressed. A key that names no field of that kind reads as
/// empty, off, or the first option.
pub struct FormValues(Vec<(&'static str, Value)>);

impl FormValues {
    fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    pub fn text(&self, key: &str) -> &str {
        match self.get(key) {
            Some(Value::Text(t)) => t,
            _ => "",
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Value::Flag(true)))
    }

    /// The index of the selected option.
    pub fn choice(&self, key: &str) -> usize {
        match self.get(key) {
            Some(Value::Choice(i)) => *i,
            _ => 0,
        }
    }
}

/// The command to run on Enter with the values, the view the form opened on, and the window.
pub type OnDone = Box<
    dyn Fn(&FormValues, Entity<Editor>, &mut Workspace, &mut Window, &mut Context<Workspace>) -> Result<(), String>,
>;

enum Slot {
    Text(Entity<Editor>),
    Toggle(bool),
    Choice(usize),
}

pub struct FormPanel {
    title: String,
    fields: Vec<Field>,
    slots: Vec<Slot>,
    /// The view the command acts on: if it is not the active one at Enter, the form just closes.
    /// `None` for a command on the window (a layout's name, a file in the side bar).
    target: Option<Entity<Editor>>,
    on_done: OnDone,
    /// What the command said was wrong, shown under the fields.
    pub error: Option<String>,
}

/// The label column's width per character (the UI font is proportional) and its least width.
const LABEL_CHAR_W: f32 = 7.;
const LABEL_MIN_W: f32 = 49.;
const GAP: f32 = 8.;

impl FormPanel {
    fn new(
        title: &str,
        fields: Vec<Field>,
        target: Option<Entity<Editor>>,
        on_done: OnDone,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let slots = fields
            .iter()
            .map(|f| match &f.kind {
                FieldKind::Text | FieldKind::Multiline => Slot::Text(cx.new(|cx| {
                    let mut e = Editor::new(EditorState::from_text(&f.initial), true, cx);
                    e.multiline = matches!(f.kind, FieldKind::Multiline);
                    e.sel(cx).select_all();
                    e
                })),
                FieldKind::Toggle(on) => Slot::Toggle(*on),
                FieldKind::Choice(options) => Slot::Choice(options.iter().position(|o| *o == f.initial).unwrap_or(0)),
            })
            .collect();
        let panel = Self { title: title.to_string(), fields, slots, target, on_done, error: None };
        if let Some(first) = panel.inputs().next() {
            first.read(cx).focus(window);
        }
        panel
    }

    /// What the form asks, as its first line shows it.
    #[cfg(test)]
    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    pub(crate) fn inputs(&self) -> impl Iterator<Item = &Entity<Editor>> {
        self.slots.iter().filter_map(|s| match s {
            Slot::Text(e) => Some(e),
            _ => None,
        })
    }

    fn values(&self, cx: &App) -> FormValues {
        FormValues(
            (self.fields.iter().zip(&self.slots))
                .map(|(f, slot)| {
                    let value = match slot {
                        Slot::Text(e) => Value::Text(e.read(cx).text(cx)),
                        Slot::Toggle(on) => Value::Flag(*on),
                        Slot::Choice(i) => Value::Choice(*i),
                    };
                    (f.key, value)
                })
                .collect(),
        )
    }

    /// A click on field `i`'s button: a toggle flips, a choice moves on.
    fn flip(&mut self, i: usize) {
        match (&mut self.slots[i], &self.fields[i].kind) {
            (Slot::Toggle(on), _) => *on ^= true,
            (Slot::Choice(n), FieldKind::Choice(options)) => *n = (*n + 1) % options.len().max(1),
            _ => {}
        }
    }

    pub fn render(&self, cx: &mut Context<Workspace>) -> AnyElement {
        let label_w =
            (self.fields.iter()).map(|f| f.label.chars().count() as f32 * LABEL_CHAR_W).fold(LABEL_MIN_W, f32::max);
        let row = || div().flex().flex_row().items_center().gap(px(GAP));
        // a toggle or a choice: a click flips it, the keys stay in the text field
        let control = |i: usize, key: &'static str, el: gpui::Stateful<gpui::Div>| {
            el.debug_selector(|| key.to_string())
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(move |ws, _: &ClickEvent, _, cx| {
                    if let Some(Panel::Form(p)) = &mut ws.panel {
                        p.flip(i);
                    }
                    cx.notify();
                }))
        };
        let rows = (self.fields.iter().zip(&self.slots)).enumerate().map(|(i, (f, slot))| {
            let field = match (slot, &f.kind) {
                (Slot::Text(e), _) => kit::field()
                    .flex_1()
                    .h(px(if matches!(f.kind, FieldKind::Multiline) { 72. } else { 26. }))
                    .child(div().size_full().py(px(4.)).px(px(6.)).child(e.clone()))
                    .into_any_element(),
                // a checkbox, as Zed draws one
                (Slot::Toggle(on), _) => control(
                    i,
                    f.key,
                    div()
                        .id(("form-field", i))
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(kit::RADIUS))
                        .border_1()
                        .cursor_pointer()
                        .map(|d| match *on {
                            true => d
                                .bg(ui(theme::ACCENT))
                                .border_color(ui(theme::ACCENT))
                                .child(icon(Icon::Check, 12.).text_color(gpui::white())),
                            false => d
                                .bg(ui(theme::WIDGET_BG))
                                .border_color(ui(theme::BORDER))
                                .hover(|s| s.border_color(ui(theme::ICON_MUTED))),
                        }),
                )
                .into_any_element(),
                // a choice shows its option and steps to the next on a click
                (Slot::Choice(n), FieldKind::Choice(options)) => {
                    let shown = options.get(*n).copied().unwrap_or_default();
                    control(
                        i,
                        f.key,
                        kit::button(("form-field", i), shown)
                            .child(icon(Icon::ChevronDown, 12.).text_color(ui(theme::ICON_MUTED))),
                    )
                    .into_any_element()
                }
                _ => div().into_any_element(),
            };
            row()
                .child(
                    div()
                        .w(px(label_w))
                        .flex()
                        .justify_end()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_color(ui(theme::TEXT_MUTED))
                        .child(f.label),
                )
                .child(field)
        });
        div()
            .key_context("FormPanel")
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .bg(ui(theme::PANEL_BG))
            .border_t_1()
            .border_color(ui(theme::BORDER))
            .text_color(ui(theme::UI_FG))
            .pl(px(10.))
            .pr(px(38.))
            .py(px(8.))
            .gap(px(6.))
            .when(!self.title.is_empty(), |d| {
                d.child(
                    div()
                        .pl(px(label_w + GAP))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(self.title.clone()),
                )
            })
            .children(rows)
            .when_some(self.error.clone(), |d, error| {
                d.child(
                    div()
                        .id("form-error")
                        .pl(px(label_w + GAP))
                        .text_size(px(11.))
                        .text_color(ui(theme::ERROR_FG))
                        .child(error),
                )
            })
            .child(
                kit::icon_button("close-form", Icon::Close, "Close  ⎋")
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .on_click(|_: &ClickEvent, window, cx| window.dispatch_action(Box::new(ClosePanel), cx)),
            )
            .into_any_element()
    }
}

impl Workspace {
    /// Asks for `fields` in the bottom panel for a command on the active view; Enter calls
    /// `on_done` with their values. Nothing opens when there is no view to act on.
    pub fn show_form(
        &mut self,
        title: &str,
        fields: Vec<Field>,
        on_done: OnDone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.active_editor() else {
            return;
        };
        self.cancel_quick_panel(cx);
        let panel = FormPanel::new(title, fields, Some(target), on_done, window, cx);
        self.show_panel(Panel::Form(panel), cx);
        // the Find panel's outlines go with it
        self.refresh_matches(cx);
    }

    /// `show_form` for a command on the window, which needs no view: `on_done` gets the
    /// form's first field as its editor.
    pub fn show_window_form(
        &mut self,
        title: &str,
        fields: Vec<Field>,
        on_done: OnDone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_quick_panel(cx);
        let panel = FormPanel::new(title, fields, None, on_done, window, cx);
        self.show_panel(Panel::Form(panel), cx);
        self.refresh_matches(cx);
    }

    /// A form of one Text field, keyed `"text"`.
    pub fn show_input(
        &mut self,
        label: &'static str,
        initial: &str,
        on_done: OnDone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field = Field::text("text", label, initial);
        self.show_form("", vec![field], on_done, window, cx);
    }

    pub(crate) fn submit_form(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        // out of the workspace while the command runs, so it can take `&mut self`
        let mut panel = match self.panel.take() {
            Some(Panel::Form(p)) => p,
            other => {
                self.panel = other;
                return;
            }
        };
        if panel.target.is_some() && self.active_editor() != panel.target {
            return self.close_panel(&ClosePanel, window, cx);
        }
        let values = panel.values(cx);
        let target = (panel.target.clone()).or_else(|| panel.inputs().next().cloned());
        let Some(target) = target else {
            return self.close_panel(&ClosePanel, window, cx);
        };
        let done = (panel.on_done)(&values, target, self, window, cx);
        // a command may open its own panel; that one stays
        if self.panel.is_some() {
            return;
        }
        match done {
            Ok(()) => self.close_panel(&ClosePanel, window, cx),
            Err(message) => {
                panel.error = Some(message);
                self.panel = Some(Panel::Form(panel));
                cx.notify();
            }
        }
    }

    forward_actions! {
    pub(crate) next_form_field(NextField, window, cx) => step_form_field(1, window, cx);
    pub(crate) prev_form_field(PrevField, window, cx) => step_form_field(-1, window, cx);
    }

    /// Tab moves between the text fields only, wrapping.
    fn step_form_field(&self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Panel::Form(p)) = &self.panel else {
            return;
        };
        let inputs: Vec<_> = p.inputs().collect();
        let at = inputs.iter().position(|e| e.read(cx).is_focused(window));
        let next = at.map_or(0, |i| (i as isize + step).rem_euclid(inputs.len() as isize) as usize);
        if let Some(e) = inputs.get(next) {
            e.read(cx).focus(window);
        }
    }
}
