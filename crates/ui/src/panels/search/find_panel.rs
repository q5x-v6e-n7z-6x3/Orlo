//! Sublime's bottom Find / Replace panel.

use crate::chrome::icons::Icon;
use crate::chrome::theme::{self, ui};
use crate::chrome::ui as kit;
use crate::editing::text::{EditorState, FindOpts};
use crate::editor::Editor;
use crate::workspace::{self, Workspace};
use gpui::{
    actions, div, prelude::*, px, Action, AnyElement, App, ClickEvent, Context, Div, Entity, KeyBinding, MouseButton,
    Stateful, Window,
};
use orlo_editor::search::model::SyntaxMask;

actions!(find_panel, [FocusNext, ReplaceOne]);

/// Bound after the editor's bindings so they win inside the panel's inputs.
pub fn key_bindings() -> Vec<KeyBinding> {
    let ctx = Some("FindPanel > Editor");
    vec![
        KeyBinding::new("enter", workspace::FindNext, ctx),
        KeyBinding::new("shift-enter", workspace::FindPrev, ctx),
        KeyBinding::new("alt-enter", workspace::FindAll, ctx),
        KeyBinding::new("cmd-alt-enter", workspace::ReplaceAll, ctx),
        KeyBinding::new("cmd-alt-r", workspace::ToggleFindRegex, ctx),
        KeyBinding::new("cmd-alt-c", workspace::ToggleFindCase, ctx),
        KeyBinding::new("cmd-alt-w", workspace::ToggleFindWord, ctx),
        KeyBinding::new("cmd-alt-a", workspace::TogglePreserveCase, ctx),
        KeyBinding::new("cmd-alt-s", workspace::ToggleFindSelection, ctx),
        KeyBinding::new("up", workspace::PreviousFindQuery, ctx),
        KeyBinding::new("down", workspace::NextFindQuery, ctx),
        KeyBinding::new("alt-down", workspace::ShowFieldHistory, ctx),
        KeyBinding::new("escape", workspace::ClosePanel, ctx),
        KeyBinding::new("tab", FocusNext, ctx),
        KeyBinding::new("cmd-alt-e", ReplaceOne, None),
    ]
}

pub struct FindPanel {
    pub query: Entity<Editor>,
    pub replace: Entity<Editor>,
    pub show_replace: bool,
    pub opts: FindOpts,
    /// Replacements take the case of what they replace.
    pub preserve_case: bool,
    /// Sublime's "highlight matches" toggle: outline every match while the panel is open.
    pub highlight: bool,
    /// The original selections, independent of the selections made by incremental find.
    pub scope: Option<(gpui::EntityId, Vec<std::ops::Range<usize>>)>,
    pub in_selection: bool,
    pub error: Option<String>,
    pub syntax: SyntaxMask,
    pub exclude_syntax: bool,
    pub(crate) syntax_search: Option<super::find::SyntaxSearch>,
}

/// A field's height: one row of text and its padding, growing a row a line up to
/// `FIELD_ROWS`; past that it scrolls.
pub fn field_height(e: &Entity<Editor>, cx: &App) -> f32 {
    let rows = e.read(cx).doc(cx).line_count().clamp(1, FIELD_ROWS);
    26. + (rows - 1) as f32 * 15.
}

/// The most rows a multiline Find or Replace field shows.
pub const FIELD_ROWS: usize = 5;

/// Space between the panel's controls, and the width of the buttons' column, so the Replace
/// field lines up under the Find field.
const GAP: f32 = 6.;
const CONTROLS_W: f32 = 208.;

/// `el` dispatching `action` when clicked; the input keeps the keys: the window would focus
/// itself on the press.
pub(super) fn dispatching(el: Stateful<Div>, action: impl Action) -> Stateful<Div> {
    el.on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |_: &ClickEvent, window, cx| window.dispatch_action(action.boxed_clone(), cx))
}

/// The Find and Find in Files panels' surface: their rows under one another, the close
/// button at the top right.
pub(super) fn panel_frame(key_context: &'static str, close_id: &'static str) -> Div {
    div()
        .key_context(key_context)
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
        .py(px(7.))
        .gap(px(GAP))
        .child(
            kit::icon_button(close_id, Icon::Close, "Close  ⎋")
                .absolute()
                .top(px(9.))
                .right(px(8.))
                .on_click(|_: &ClickEvent, window, cx| window.dispatch_action(Box::new(workspace::ClosePanel), cx)),
        )
}

/// A panel's problem with its search, under its fields.
pub(super) fn error_line(id: &'static str, error: String) -> Stateful<Div> {
    div().id(id).text_size(px(11.)).text_color(ui(theme::ERROR_FG)).child(error)
}

/// A search field with its options inside and an optional empty-state placeholder.
pub(super) fn search_field(
    e: &Entity<Editor>,
    placeholder: Option<&'static str>,
    options: Vec<Stateful<Div>>,
    cx: &App,
) -> Div {
    kit::field()
        .when(placeholder.is_some(), |d| d.relative())
        .flex_1()
        .min_w_0()
        .h(px(field_height(e, cx)))
        .flex()
        .flex_row()
        .items_start()
        .child(div().flex_1().min_w_0().h_full().py(px(4.)).px(px(6.)).child(e.clone()))
        .when_some(placeholder.filter(|_| e.read(cx).doc(cx).is_empty()), |d, placeholder| {
            d.child(
                div()
                    .absolute()
                    .left(px(7.))
                    .top(px(4.))
                    .font_family(theme::EDITOR_FONT)
                    .text_size(px(theme::EDITOR_FONT_SIZE))
                    .text_color(ui(theme::PLACEHOLDER_FG))
                    .child(placeholder),
            )
        })
        .child(div().flex().flex_none().pt(px(1.)).pr(px(1.)).children(options))
}

impl FindPanel {
    pub fn new(
        show_replace: bool,
        initial: &str,
        replacement: &str,
        opts: FindOpts,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let query = cx.new(|cx| {
            let mut e = Editor::new(EditorState::from_text(initial), true, cx);
            e.multiline = true;
            e.sel(cx).select_all();
            e
        });
        let replace = cx.new(|cx| {
            let mut e = Editor::new(EditorState::from_text(replacement), true, cx);
            e.multiline = true;
            e
        });
        // a replacement is checked against the pattern as it is typed
        Editor::on_text_change(&replace, cx, |ws, cx| {
            ws.refresh_matches(cx);
            // a line more or less grows or shrinks the field
            cx.notify();
        })
        .detach();
        Editor::on_text_change(&query, cx, |ws, cx| {
            ws.refresh_matches(cx);
            ws.find_as_you_type(cx);
            cx.notify();
        })
        .detach();
        query.read(cx).focus(window);
        Self {
            query,
            replace,
            show_replace,
            opts,
            preserve_case: false,
            highlight: true,
            scope: None,
            in_selection: false,
            error: None,
            syntax: SyntaxMask::Any,
            exclude_syntax: false,
            syntax_search: None,
        }
    }

    /// In Selection, while there is a selection to search in.
    pub(super) fn toggle_in_selection(&mut self) {
        if (self.scope.as_ref()).is_some_and(|(_, ranges)| !ranges.is_empty()) {
            self.in_selection ^= true;
        }
    }

    pub fn render(&self, cx: &mut Context<Workspace>) -> AnyElement {
        let toggle = |id: &'static str, i: Icon, tip: &'static str, on: bool, flip: fn(&mut FindPanel)| {
            kit::icon_toggle(id, i, tip, on)
                .debug_selector(|| id.to_string())
                // the input keeps the keys: the window would focus itself on the press
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(move |ws, _: &ClickEvent, _, cx| {
                    if let Some(p) = ws.find_panel_mut() {
                        flip(p);
                    }
                    ws.refresh_matches(cx);
                    ws.find_as_you_type(cx);
                    cx.notify();
                }))
        };
        // a field and the options that belong to its text, inside it at the right (Zed's)
        let field = |e, options| search_field(e, None, options, cx);
        let controls = || {
            div().w(px(CONTROLS_W)).flex_none().flex().flex_row().items_center().justify_end().gap(px(2.)).h(px(26.))
        };
        let row = || div().flex().flex_row().items_start().gap(px(GAP));
        let o = self.opts;
        panel_frame("FindPanel", "close-find")
            .child(
                row()
                    .child(field(
                        &self.query,
                        vec![
                            toggle("case", Icon::CaseSensitive, "Case Sensitive  ⌥⌘C", o.case_sensitive, |p| {
                                p.opts.case_sensitive ^= true
                            }),
                            toggle("word", Icon::WholeWord, "Whole Word  ⌥⌘W", o.whole_word, |p| {
                                p.opts.whole_word ^= true
                            }),
                            toggle("regex", Icon::Regex, "Regular Expression  ⌥⌘R", o.regex, |p| {
                                p.opts.regex ^= true
                            }),
                        ],
                    ))
                    .child(
                        controls()
                            .child(toggle("wrap", Icon::Wrap, "Wrap Around", o.wrap, |p| p.opts.wrap ^= true))
                            .child(toggle(
                                "selection",
                                Icon::InSelection,
                                "In Selection  ⌥⌘S",
                                self.in_selection,
                                FindPanel::toggle_in_selection,
                            ))
                            .child(toggle("highlight", Icon::Highlight, "Highlight Matches", self.highlight, |p| {
                                p.highlight ^= true
                            }))
                            .child(div().w(px(6.)))
                            .child(dispatching(
                                kit::icon_button("find-prev", Icon::ChevronLeft, "Find Previous  ⇧↩"),
                                workspace::FindPrev,
                            ))
                            .child(dispatching(
                                kit::icon_button("find-next", Icon::ChevronRight, "Find Next  ↩"),
                                workspace::FindNext,
                            ))
                            .child(dispatching(kit::button("find-all", "Find All").ml(px(4.)), workspace::FindAll)),
                    ),
            )
            .child(
                row()
                    .child(
                        kit::button("syntax-filter", self.syntax.label())
                            .debug_selector(|| "syntax-filter".into())
                            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                            .on_click(cx.listener(|ws, _: &ClickEvent, _, cx| {
                                if let Some(p) = ws.find_panel_mut() {
                                    p.syntax = p.syntax.next();
                                    if p.syntax == SyntaxMask::Any {
                                        p.syntax_search = None;
                                    }
                                }
                                ws.refresh_matches(cx);
                                ws.find_as_you_type(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        kit::button("syntax-exclude", if self.exclude_syntax { "Exclude" } else { "Include" })
                            .debug_selector(|| "syntax-exclude".into())
                            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                            .on_click(cx.listener(|ws, _: &ClickEvent, _, cx| {
                                if let Some(p) = ws.find_panel_mut() {
                                    p.exclude_syntax ^= true;
                                }
                                ws.refresh_matches(cx);
                                ws.find_as_you_type(cx);
                                cx.notify();
                            })),
                    ),
            )
            .when(self.show_replace, |d| {
                d.child(
                    row()
                        .child(field(
                            &self.replace,
                            vec![toggle(
                                "preserve-case",
                                Icon::PreserveCase,
                                "Preserve Case  ⌥⌘A",
                                self.preserve_case,
                                |p| p.preserve_case ^= true,
                            )],
                        ))
                        .child(
                            controls()
                                .gap(px(4.))
                                .child(dispatching(kit::button("replace-one", "Replace"), ReplaceOne))
                                .child(dispatching(kit::button("replace-all", "Replace All"), workspace::ReplaceAll)),
                        ),
                )
            })
            .when_some(self.error.clone(), |d, error| d.child(error_line("find-error", error)))
            .into_any_element()
    }
}
