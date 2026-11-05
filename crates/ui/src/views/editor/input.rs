//! IME and text input through gpui's `EntityInputHandler`.

use super::*;

use crate::editing::text::{CharOffset, Utf16Offset};
use gpui::{point, px, size, Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window};
use std::ops::Range;

/// A range as gpui's input handler gives it: UTF-16 code units of the text.
fn utf16(r: Range<usize>) -> Range<Utf16Offset> {
    Utf16Offset(r.start)..Utf16Offset(r.end)
}

/// The source after an overtype/virtual preview stays in the rope until commit. Explicit
/// native ranges may differ from the view selections restored on cancellation.
pub(super) struct OvertypeComposition {
    revision: u64,
    targets: Vec<Selection>,
    original_selections: Vec<Selection>,
    primary: usize,
    selections: Vec<Selection>,
    padding: Vec<Range<usize>>,
}

pub(crate) struct RecoveryComposition {
    pub removed: Vec<Range<usize>>,
    pub selections: Vec<(usize, usize)>,
}

#[cfg(test)]
struct LiteralKeyboardInput(bool);
#[cfg(test)]
impl gpui::Global for LiteralKeyboardInput {}
#[cfg(test)]
struct LiteralNativeKey(bool);
#[cfg(test)]
impl gpui::Global for LiteralNativeKey {}
#[cfg(test)]
struct LiteralNativeText(Option<String>);
#[cfg(test)]
impl gpui::Global for LiteralNativeText {}

fn literal_keyboard_input(cx: &App) -> bool {
    #[cfg(not(test))]
    {
        let _ = cx;
        crate::app::macos::direct_keyboard_input()
    }
    #[cfg(test)]
    {
        cx.try_global::<LiteralKeyboardInput>().is_none_or(|source| source.0)
    }
}

fn literal_native_text(fallback: &str, cx: &App) -> Option<String> {
    #[cfg(not(test))]
    {
        let _ = (fallback, cx);
        crate::app::macos::literal_keyboard_text()
    }
    #[cfg(test)]
    {
        if cx.try_global::<LiteralNativeKey>().is_some_and(|key| !key.0) {
            return None;
        }
        cx.try_global::<LiteralNativeText>().map_or_else(|| Some(fallback.to_string()), |text| text.0.clone())
    }
}

/// Subscriptions exist only while armed; ordinary typing adds no key interceptor.
pub(super) struct LiteralNext {
    document: gpui::EntityId,
    version: u64,
    overtype: bool,
    peers: Vec<WeakEntity<Editor>>,
    _keys: Subscription,
    _blur: Subscription,
    _source: Subscription,
}

pub(crate) fn unicode_scalar(value: &str) -> Result<char, String> {
    let value = value.trim();
    let hex = value.strip_prefix("U+").or_else(|| value.strip_prefix("u+")).unwrap_or(value);
    let invalid = || "Enter one Unicode scalar in hex, such as U+1F600 (no surrogates)".to_string();
    if hex.is_empty() || hex.len() > 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).ok_or_else(invalid)
}

impl Editor {
    pub(crate) fn character_input_admission(&self, cx: &Context<Self>) -> Result<(), String> {
        if self.single_line || !self.input_enabled || self.doc(cx).read_only || self.doc(cx).edits_blocked() {
            return Err("Document is read only".into());
        }
        if !self.marked.is_empty() {
            return Err("Finish or cancel text composition before inserting a character".into());
        }
        Ok(())
    }

    pub(crate) fn try_insert_unicode_character(&mut self, value: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let scalar = unicode_scalar(value)?;
        self.catch_up(cx);
        self.character_input_admission(cx)?;
        self.literal_next = None;
        let mut bytes = [0; 4];
        let text = scalar.encode_utf8(&mut bytes);
        self.edit(cx, |s| {
            s.end_typing();
            s.insert(text);
            s.end_typing();
        });
        Ok(())
    }

    pub(crate) fn insert_unicode_character(
        &mut self,
        a: &InsertUnicodeCharacter,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.try_insert_unicode_character(&a.value, cx) {
            self.status(error, cx);
        }
    }

    fn literal_current(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.literal_next.as_ref().is_some_and(|armed| {
            armed.document == self.document.entity_id()
                && armed.version == self.doc(cx).version
                && armed.overtype == self.state.overtype
                && self.is_focused(window)
                && self.character_input_admission(cx).is_ok()
                && armed.peers.iter().all(|peer| peer.upgrade().is_none_or(|peer| peer.read(cx).marked.is_empty()))
        })
    }

    pub(super) fn stale_literal_input(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.literal_next.is_some() && !self.literal_current(window, cx) {
            self.literal_next = None;
            self.status("Literal input canceled because the editor changed", cx);
            return true;
        }
        false
    }

    /// Workspace captures peers before borrowing this editor; input must never reenter Workspace.
    pub(crate) fn set_literal_input_peers(&mut self, peers: Vec<Entity<Editor>>, cx: &Context<Self>) {
        let me = cx.entity_id();
        if let Some(armed) = &mut self.literal_next {
            armed.peers = peers
                .into_iter()
                .filter(|peer| peer.entity_id() != me && peer.read(cx).document == self.document)
                .map(|peer| peer.downgrade())
                .collect();
        }
    }

    pub(crate) fn take_literal_input(&mut self, window: &Window, cx: &Context<Self>) -> bool {
        let current = self.literal_current(window, cx);
        self.literal_next = None;
        current
    }

    pub(crate) fn insert_literal_next_key(
        &mut self,
        _: &InsertLiteralNextKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.literal_next = None;
        self.catch_up(cx);
        if let Err(error) = self.character_input_admission(cx) {
            return self.status(error, cx);
        }
        if !literal_keyboard_input(cx) {
            return self.status("Insert Literal Next Key yields to the active native input method", cx);
        }
        if !self.is_focused(window) {
            return;
        }
        let (weak, here) = (cx.weak_entity(), window.window_handle());
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != here {
                return;
            }
            let handled = weak
                .update(cx, |e, cx| {
                    if e.stale_literal_input(window, cx) {
                        return true;
                    }
                    if !literal_keyboard_input(cx) {
                        e.literal_next = None;
                        return false;
                    }
                    let key = &event.keystroke;
                    if key.key == "escape" {
                        e.literal_next = None;
                        return true;
                    }
                    // Native IME owns dead keys and marked text; shortcut modifiers never become text.
                    if key.modifiers.control || key.modifiers.platform || key.modifiers.alt || key.modifiers.function {
                        e.literal_next = None;
                        return false;
                    }
                    let Some(fallback) = key.key_char.as_deref().filter(|t| !t.is_empty()) else {
                        e.literal_next = None;
                        return false;
                    };
                    let Some(mut text) = literal_native_text(fallback, cx) else {
                        e.literal_next = None;
                        return false;
                    };
                    // GPUI identifies physical Return/keypad Enter as `enter` + LF; AppKit
                    // supplies CR/Enter. Normalize only that qualified newline event.
                    if key.key == "enter" && fallback == "\n" && matches!(text.as_str(), "\r" | "\u{3}") {
                        text = "\n".into();
                    }
                    if text.is_empty() || text.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\n')) {
                        e.literal_next = None;
                        return false;
                    }
                    e.replace_text_in_range(None, &text, window, cx);
                    true
                })
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        });
        let blur = cx.on_blur(&self.focus_handle, window, |e, _, _| e.literal_next = None);
        let weak = cx.weak_entity();
        let source = cx.on_keyboard_layout_change(move |cx| {
            weak.update(cx, |e, _| e.literal_next = None).ok();
        });
        self.literal_next = Some(LiteralNext {
            document: self.document.entity_id(),
            version: self.doc(cx).version,
            overtype: self.state.overtype,
            peers: Vec::new(),
            _keys: keys,
            _blur: blur,
            _source: source,
        });
        self.status("Insert Literal Next Key: type text or press Escape", cx);
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.chars_in(utf16(range_utf16), cx);
        actual_range.replace(self.utf16_in(r.clone(), cx));
        Some(self.doc(cx).rope.slice(r).to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, cx: &mut Context<Self>) -> Option<UTF16Selection> {
        let s = *self.state.selections.get(self.state.primary())?;
        Some(UTF16Selection { range: self.utf16_in(s.range(), cx), reversed: s.head_point() < s.anchor_point() })
    }

    fn marked_text_range(&self, _: &mut Window, cx: &mut Context<Self>) -> Option<Range<usize>> {
        let r = self.marked.get(self.state.primary())?;
        Some(self.utf16_in(r.clone(), cx))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.catch_up(cx);
        if self.valid_overtype_composition(cx) {
            let text = self.doc(cx).rope.slice(self.marked[self.state.primary()].clone()).to_string();
            self.replace_text_in_range(None, &text, window, cx);
        }
        self.overtype_composition = None;
        self.marked.clear();
        // Remove composition underlines and restore warning overlays immediately.
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.stale_literal_input(window, cx) {
            return;
        }
        let literal = self.take_literal_input(window, cx);
        if !self.input_enabled || self.doc(cx).edits_blocked() {
            return;
        }
        self.catch_up(cx);
        self.validate_signature_context(window, cx);
        let previous_signature = self
            .signature_help
            .as_ref()
            .map(|popup| (popup.help.raw.clone(), popup.help.active))
            .or_else(|| self.signature_pending.as_ref().and_then(|pending| pending.context.clone()));
        let continuing = previous_signature.is_some() || self.signature_pending.is_some();
        let replacing = range_utf16.is_some();
        let ranges = self.input_targets(range_utf16, cx);
        let current_targets = ranges.iter().zip(&self.state.selections).all(|(r, s)| *r == s.range());
        let overtype = self.take_overtype_composition(&ranges, cx);
        if !self.single_line {
            let replace = (replacing && self.marked.is_empty()).then(|| {
                let range = &ranges[self.state.primary()];
                let caret = self.state.cursor() as isize;
                [range.start as isize - caret, range.end as isize - caret]
            });
            if literal {
                crate::config::customization::record(
                    crate::keybindings::Command {
                        command: "editor::InsertLiteralText".into(),
                        args: serde_json::json!({"text": text, "replace": replace}),
                    },
                    cx,
                );
            } else {
                crate::config::customization::record_text(text, replace, cx);
            }
        }
        let composing = !std::mem::take(&mut self.marked).is_empty();
        let text = if self.fixed_rows() { text.replace('\n', " ") } else { text.to_string() };
        let plain_quote = text == "'" && !self.highlighter.pairs_single_quote();
        self.change(cx, |s| {
            if let Some(original) = &overtype {
                s.remove_composition(&ranges, &original.padding);
                s.reselect(if text.is_empty() {
                    original.original_selections.clone()
                } else {
                    original.targets.clone()
                });
            } else if composing {
                // the commit types its own text where the composed text was (typed over it, a
                // dead key's quote would wrap the marked one in a pair)
                s.compose(&ranges, "", false);
            } else if replacing && !current_targets {
                let sels = ranges.iter().zip(&s.view.selections).map(|(r, original)| Selection {
                    rectangular: original.rectangular,
                    ..Selection::new(r.start, r.end)
                });
                s.reselect(sels.collect());
            }
            if text.is_empty() && overtype.is_some() {
                return;
            }
            if literal {
                s.end_typing();
                s.type_literal(&text);
                s.end_typing();
            } else if replacing && overtype.is_none() && s.view.overtype {
                s.insert(&text)
            } else if plain_quote {
                s.type_literal(&text)
            } else {
                s.type_text(&text)
            }
        });
        self.after_edit(cx);
        self.typed(&text, window, cx);
        let trigger = text.chars().count() == 1
            && (self.signature_triggers.0.contains(&text)
                || (previous_signature.is_some() && self.signature_triggers.1.contains(&text)));
        if !self.single_line && self.focus_handle.is_focused(window) && !self.completing(cx) && (trigger || continuing)
        {
            self.signature_pending = Some(Self::signature_pending_context(previous_signature.clone()));
            cx.emit(EditorEvent::SignatureInput(text, previous_signature, trigger));
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.stale_literal_input(window, cx) {
            return;
        }
        if !self.input_enabled || self.doc(cx).edits_blocked() {
            return;
        }
        // Arming never changes native composition ranges or steals its candidate keys.
        self.literal_next = None;
        // the list is hidden while marked text is composed
        self.completion = None;
        self.catch_up(cx);
        let mut ranges = self.input_targets(range_utf16, cx);
        let start = self.marked.is_empty();
        let mut overtype = self.take_overtype_composition(&ranges, cx);
        let current_targets = ranges.iter().zip(&self.state.selections).all(|(r, s)| *r == s.range());
        let virtual_start = start && current_targets && self.state.selections.iter().any(Selection::has_virtual);
        if start && (self.state.overtype || virtual_start) {
            overtype = Some(OvertypeComposition {
                revision: self.doc(cx).version,
                targets: if current_targets {
                    self.state.selections.clone()
                } else {
                    ranges
                        .iter()
                        .zip(&self.state.selections)
                        .map(|(r, original)| Selection {
                            rectangular: original.rectangular,
                            ..Selection::new(r.start, r.end)
                        })
                        .collect()
                },
                original_selections: self.state.selections.clone(),
                primary: self.state.primary(),
                selections: Vec::new(),
                padding: Vec::new(),
            });
            for r in &mut ranges {
                r.end = r.start;
            }
        }
        let placed = self.change(cx, |s| {
            if virtual_start && !new_text.is_empty() {
                let (placed, padding) = s.compose_virtual(new_text);
                overtype.as_mut().unwrap().padding = padding;
                placed
            } else if new_text.is_empty() && overtype.is_some() && !start {
                s.remove_composition(&ranges, &overtype.as_ref().unwrap().padding);
                ranges.iter().map(|r| r.start..r.start).collect()
            } else if let Some(original) = &mut overtype {
                s.compose_tracking_padding(&ranges, new_text, start, &mut original.padding)
            } else {
                s.compose(&ranges, new_text, start)
            }
        });
        self.marked = if new_text.is_empty() { Vec::new() } else { placed.clone() };
        // the IME's caret (or selection) inside its text, at every caret
        let (a, b) = match new_selected_range_utf16 {
            Some(sel) => {
                let to_char = |u: usize| {
                    let mut units = 0;
                    new_text
                        .chars()
                        .take_while(|c| {
                            let keep = units < u;
                            units += c.len_utf16();
                            keep
                        })
                        .count()
                };
                (to_char(sel.start), to_char(sel.end))
            }
            None => {
                let n = new_text.chars().count();
                (n, n)
            }
        };
        let sels = if new_text.is_empty() {
            match &overtype {
                Some(original) => original.original_selections.clone(),
                None => placed
                    .iter()
                    .zip(&self.state.selections)
                    .map(|(r, original)| Selection { rectangular: original.rectangular, ..Selection::caret(r.start) })
                    .collect(),
            }
        } else {
            placed
                .iter()
                .enumerate()
                .map(|(i, r)| Selection {
                    rectangular: self.state.selections.get(i).is_some_and(|sel| sel.rectangular),
                    ..Selection::new(r.start + a, r.start + b)
                })
                .collect()
        };
        self.sel(cx).reselect(sels);
        self.overtype_composition = overtype.filter(|_| !new_text.is_empty()).map(|mut original| {
            original.revision = self.doc(cx).version;
            original.selections = self.state.selections.clone();
            original
        });
        self.after_edit(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.char_bounds(self.chars_in(utf16(range_utf16), cx).start, cx)
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_for_position(point, cx)?;
        Some(self.utf16_in(offset..offset, cx).start)
    }
}

impl Editor {
    fn valid_overtype_composition(&self, cx: &App) -> bool {
        self.overtype_composition.as_ref().is_some_and(|original| {
            original.revision == self.doc(cx).version
                && !self.marked.is_empty()
                && self.marked.len() == original.targets.len()
                && original.primary == self.state.primary()
                && original.selections == self.state.selections
        })
    }

    /// Recovery excludes only a valid provisional representation. Live native input/history
    /// remain untouched; stale targets already have ordinary insertion semantics.
    pub(crate) fn recovery_composition(&self, cx: &App) -> Option<RecoveryComposition> {
        if !self.valid_overtype_composition(cx) {
            return None;
        }
        let original = self.overtype_composition.as_ref()?;
        let mut ranges: Vec<_> =
            self.marked.iter().chain(&original.padding).filter(|r| !r.is_empty()).cloned().collect();
        ranges.sort_by_key(|r| r.start);
        let mut selections = original.original_selections.clone();
        let primary = selections.remove(original.primary);
        selections.push(primary);
        Some(RecoveryComposition {
            removed: ranges,
            selections: selections.into_iter().map(|s| (s.anchor, s.head)).collect(),
        })
    }

    fn take_overtype_composition(&mut self, ranges: &[Range<usize>], cx: &App) -> Option<OvertypeComposition> {
        let valid = self.valid_overtype_composition(cx) && ranges == self.marked;
        self.overtype_composition.take().filter(|_| valid)
    }

    /// The chars a UTF-16 range covers in the text as it is now; a code unit inside a surrogate
    /// pair is its char's.
    fn chars_in(&self, r: Range<Utf16Offset>, cx: &App) -> Range<usize> {
        let text = self.doc(cx).coords();
        text.utf16_to_char(r.start).0..text.utf16_to_char(r.end).0
    }

    /// Chars `r` of the text as it is now in UTF-16 code units, as gpui's input handler counts.
    fn utf16_in(&self, r: Range<usize>, cx: &App) -> Range<usize> {
        let text = self.doc(cx).coords();
        text.char_to_utf16(CharOffset(r.start)).0..text.char_to_utf16(CharOffset(r.end)).0
    }

    /// Where char `offset` is drawn in the last painted frame, a caret wide and a row high (the
    /// input method's bounds, and the popup's anchor); `None` before the first paint or off screen.
    pub(crate) fn char_bounds(&self, offset: usize, cx: &App) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        // the column from the text as it is now; the painted row (maybe a frame older than an
        // edit) only gives the x of that column
        let p = self.rows().row_of(self.doc(cx), offset);
        let (row, col) = (p.row.0, p.col.0);
        let (x0, shaped) = layout.rows.get(row.checked_sub(layout.first_row)?)?.as_ref()?;
        let gap =
            self.state.selections.get(self.state.primary()).filter(|s| s.head == offset).map_or(0, |s| s.head_virtual);
        let x = layout.text_left
            + layout.space_w * (*x0 + gap) as f32
            + shaped.x_for_index(byte_of(&shaped.text, col.saturating_sub(*x0)));
        let lh = self.line_height();
        let y = layout.bounds.top() + px((row as f64 * lh as f64 - layout.scroll_top) as f32);
        Some(Bounds::new(point(x, y), size(px(2.), px(lh))))
    }

    /// The ranges an input method's edit applies to, one per caret: the composition's marked
    /// ranges, or the selections. A range the input method names is about the primary caret
    /// (press-and-hold names the letter before it): the same shift applies at every caret.
    fn input_targets(&mut self, range_utf16: Option<Range<usize>>, cx: &App) -> Vec<Range<usize>> {
        if self.overtype_composition.is_some() && !self.valid_overtype_composition(cx) {
            self.overtype_composition = None;
            self.marked.clear();
        }
        if self.marked.len() != self.state.selections.len() {
            self.marked.clear();
        }
        let base: Vec<Range<usize>> = match self.marked.is_empty() {
            true => self.state.selections.iter().map(|s| s.range()).collect(),
            false => self.marked.clone(),
        };
        let Some(r) = range_utf16 else {
            return base;
        };
        let r = self.chars_in(utf16(r), cx);
        let p = &base[self.state.primary().min(base.len() - 1)];
        let (ds, de) = (r.start as isize - p.start as isize, r.end as isize - p.end as isize);
        let len = self.doc(cx).len() as isize;
        let at = |o: usize, d: isize| (o as isize + d).clamp(0, len) as usize;
        base.iter()
            .map(|b| {
                let (s, e) = (at(b.start, ds), at(b.end, de));
                s.min(e)..e.max(s)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler, TestAppContext};

    #[gpui::test]
    fn utf16_ranges_meet_chars_at_every_kind_of_char(cx: &mut TestAppContext) {
        // ASCII, precomposed and combining accents, outside the basic plane, tabs, line breaks
        let text = "ab café e\u{301}\n😀\t𝄞x\n\n中文 🦀";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(text), false, cx));
        let units: Vec<usize> = (text.char_indices().map(|(b, _)| b))
            .chain([text.len()])
            .map(|b| text[..b].encode_utf16().count())
            .collect();
        e.update_in(cx, |e, window, cx| {
            for (o, &u) in units.iter().enumerate() {
                e.sel(cx).set_cursor(o);
                let sel = e.selected_text_range(false, window, cx).unwrap();
                assert_eq!(sel.range, u..u, "caret at char {o}");
            }
            let total = *units.last().unwrap();
            for a in 0..=total {
                for b in a..=total {
                    let mut actual = None;
                    let got = e.text_for_range(a..b, &mut actual, window, cx).unwrap();
                    // a unit inside a surrogate pair is its char's: the range read is the
                    // whole chars it starts in, up to the char its end is in
                    let actual = actual.unwrap();
                    let (s, t) = (actual.start, actual.end);
                    assert!(units.contains(&s) && units.contains(&t), "{a}..{b}: {s}..{t}");
                    assert!(s <= a && s + 1 >= a && t <= b && t + 1 >= b, "{a}..{b}: {s}..{t}");
                    let chars = |u: usize| units.iter().position(|&x| x == u).unwrap();
                    let want: String = text.chars().skip(chars(s)).take(chars(t) - chars(s)).collect();
                    assert_eq!(got, want, "{a}..{b}");
                }
            }
        });
    }
}

#[cfg(test)]
mod overtype_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn overtype_native_ime_updates_and_commit_preserve_source_at_every_caret(cx: &mut TestAppContext) {
        let original = "e\u{301}X\r\n😀YZ";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(original), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            e.sel(cx).set_selections(vec![Selection::caret(0), Selection::caret(5)]);
            for preview in ["k", "かな", "😀", "界"] {
                e.replace_and_mark_text_in_range(None, preview, None, window, cx);
                assert_eq!(e.text(cx), format!("{preview}e\u{301}X\r\n{preview}😀YZ"));
                assert_eq!(e.marked.len(), 2);
            }
            let marked = e.marked_text_range(window, cx);
            e.replace_text_in_range(marked, "界", window, cx);
            assert_eq!(e.text(cx), "界X\r\n界YZ");
            assert!(e.marked.is_empty());
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), original);
            e.edit(cx, |s| s.redo());
            assert_eq!(e.text(cx), "界X\r\n界YZ");
        });
    }

    #[gpui::test]
    fn overtype_ime_selected_source_survives_updates_and_both_cancel_paths(cx: &mut TestAppContext) {
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("abc😀tail"), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            for cancel_with_mark in [true, false] {
                e.sel(cx).set_selection(1, 4);
                e.replace_and_mark_text_in_range(None, "😀", Some(0..2), window, cx);
                assert_eq!(e.text(cx), "a😀bc😀tail");
                assert_eq!(e.selected_text_range(false, window, cx).unwrap().range, 1..3);
                e.replace_and_mark_text_in_range(None, "かな", None, window, cx);
                assert_eq!(e.text(cx), "aかなbc😀tail");
                if cancel_with_mark {
                    e.replace_and_mark_text_in_range(None, "", None, window, cx);
                } else {
                    e.replace_text_in_range(None, "", window, cx);
                }
                assert_eq!(e.text(cx), "abc😀tail");
                assert_eq!(e.state.selections, vec![Selection::new(1, 4)]);
                assert!(e.marked.is_empty());
            }
            e.replace_and_mark_text_in_range(None, "é", None, window, cx);
            e.unmark_text(window, cx);
            assert_eq!(e.text(cx), "aétail");
            assert!(e.marked.is_empty());
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "abc😀tail");
        });
    }

    #[gpui::test]
    fn overtype_native_quotes_selections_and_explicit_utf16_replacements(cx: &mut TestAppContext) {
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("a😀bc"), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            e.replace_text_in_range(Some(1..3), "long", window, cx);
            assert_eq!(e.text(cx), "alongbc");
            e.sel(cx).set_cursor(0);
            e.replace_text_in_range(None, "'", window, cx);
            assert_eq!(e.text(cx), "'longbc");
            e.sel(cx).set_selection(1, 5);
            e.replace_text_in_range(None, "(", window, cx);
            assert_eq!(e.text(cx), "'(bc");
            e.sel(cx).set_cursor(2);
            e.replace_text_in_range(Some(2..2), "INSERT", window, cx);
            assert_eq!(e.text(cx), "'(INSERTbc");
        });
    }

    #[gpui::test]
    fn overtype_composition_targets_are_invalid_after_another_view_edits(cx: &mut TestAppContext) {
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("abc"), false, cx));
        let other = e.update(cx, |e, cx| {
            let document = e.document.clone();
            let view = e.state.duplicate(e.doc(cx));
            cx.new(|cx| Editor::of_document(document, view, false, cx))
        });
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            e.sel(cx).set_selection(0, 2);
            e.replace_and_mark_text_in_range(None, "😀", None, window, cx);
            assert_eq!(e.text(cx), "😀abc");
        });
        other.update(cx, |other, cx| {
            assert!(!other.state.overtype);
            other.edit(cx, |s| {
                s.set_cursor(s.len());
                s.insert("!");
            });
        });
        e.update_in(cx, |e, window, cx| {
            e.unmark_text(window, cx);
            assert_eq!(e.text(cx), "😀abc!");
            assert!(e.marked.is_empty());
            assert!(e.overtype_composition.is_none());
        });
    }

    #[gpui::test]
    fn overtype_native_input_and_ime_commit_extend_preceding_graphemes(cx: &mut TestAppContext) {
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("abc"), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            e.sel(cx).set_cursor(1);
            e.replace_text_in_range(None, "\u{301}", window, cx);
            assert_eq!(e.text(cx), "a\u{301}bc");
            e.set_text("👨‍bc", cx);
            e.sel(cx).set_cursor(2);
            e.replace_and_mark_text_in_range(None, "👩", None, window, cx);
            e.replace_text_in_range(None, "👩", window, cx);
            assert_eq!(e.text(cx), "👨‍👩bc");
        });
    }

    #[gpui::test]
    fn overtype_changed_selection_does_not_commit_an_old_composition_target(cx: &mut TestAppContext) {
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("abcd"), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            e.sel(cx).set_selection(0, 2);
            e.replace_and_mark_text_in_range(None, "X", None, window, cx);
            e.sel(cx).set_cursor(5);
            e.unmark_text(window, cx);
            assert_eq!(e.text(cx), "Xabcd");
            assert!(e.overtype_composition.is_none());
            e.sel(cx).set_selection(1, 3);
            e.replace_and_mark_text_in_range(None, "Y", None, window, cx);
            e.sel(cx).set_cursor(6);
            e.replace_text_in_range(None, "!", window, cx);
            assert_eq!(e.text(cx), "XYabcd!");
        });
    }

    #[gpui::test]
    fn review_overtype_native_long_emoji_continuation_retains_following_source(cx: &mut TestAppContext) {
        let original = format!("👩{}z", "\u{301}".repeat(600));
        let expected = format!("👩{}\u{200d}💻z", "\u{301}".repeat(600));
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(&original), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            e.sel(cx).set_cursor(601);
            e.replace_text_in_range(None, "\u{200d}💻", window, cx);
            assert_eq!(e.text(cx), expected);
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), original);
            e.edit(cx, |s| s.redo());
            assert_eq!(e.text(cx), expected);
        });
    }

    fn assert_cancel_restores_original_selections(cx: &mut TestAppContext, marked_cancel: bool) {
        let source = "ab😀cd\nEF🦀GH";
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(source), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            // The first region in document order is backward and primary, not the final region.
            e.sel(cx).set_selections(vec![Selection::new(7, 10), Selection::new(3, 1)]);
            let original = e.state.selections.clone();
            let primary = e.state.primary();
            assert_eq!(primary, 0);
            e.replace_and_mark_text_in_range(None, "😀", Some(0..2), window, cx);
            e.replace_and_mark_text_in_range(None, "かな", None, window, cx);
            if marked_cancel {
                e.replace_and_mark_text_in_range(None, "", None, window, cx);
            } else {
                e.replace_text_in_range(None, "", window, cx);
            }
            assert_eq!(e.text(cx), source);
            assert_eq!(e.state.selections, original);
            assert_eq!(e.state.primary(), primary);
            assert!(e.marked.is_empty());
        });
    }

    #[gpui::test]
    fn review_overtype_marked_cancel_restores_backward_and_primary_selections(cx: &mut TestAppContext) {
        assert_cancel_restores_original_selections(cx, true);
    }

    #[gpui::test]
    fn review_overtype_empty_commit_restores_backward_and_primary_selections(cx: &mut TestAppContext) {
        assert_cancel_restores_original_selections(cx, false);
    }

    #[gpui::test]
    fn review_overtype_interior_native_caret_replaces_the_complete_grapheme(cx: &mut TestAppContext) {
        let original = format!("👩{}z", "\u{301}".repeat(600));
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text(&original), false, cx));
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            e.sel(cx).set_cursor(600);
            e.replace_text_in_range(None, "a", window, cx);
            assert_eq!(e.text(cx), "az");
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), original);
        });
    }

    #[gpui::test]
    fn overtype_native_read_only_working_copy_keeps_editing_history_and_save_protection(cx: &mut TestAppContext) {
        for composing in [false, true] {
            let original = "e\u{301}😀tail";
            let (e, cx) = cx.add_window_view(|_, cx| {
                let mut state = EditorState::from_text(original);
                state.doc.read_only = true;
                Editor::new(state, false, cx)
            });
            e.update_in(cx, |e, w, cx| {
                e.perform_action(&ToggleOvertype, w, cx);
                assert!(e.state.overtype);
                if composing {
                    e.replace_and_mark_text_in_range(None, "仮", None, w, cx);
                    assert_eq!(e.text(cx), format!("仮{original}"));
                    e.replace_and_mark_text_in_range(None, "かな", None, w, cx);
                    assert_eq!(e.text(cx), format!("かな{original}"));
                }
                e.replace_text_in_range(None, "界", w, cx);
                assert_eq!(e.text(cx), "界😀tail");
                assert!(e.marked.is_empty());
                e.change(cx, |s| {
                    assert_eq!(s.doc.save().unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
                });
                assert!(e.doc(cx).read_only);
                e.edit(cx, |s| s.undo());
                assert_eq!(e.text(cx), original);
                e.edit(cx, |s| s.redo());
                assert_eq!(e.text(cx), "界😀tail");
                assert!(e.state.overtype);
            });
        }
    }

    #[gpui::test]
    fn overtype_is_unavailable_in_fields_and_immutable_viewers(cx: &mut TestAppContext) {
        let (e, cx) = cx.add_window_view(|_, cx| Editor::new(EditorState::from_text("abc"), true, cx));
        e.update_in(cx, |e, window, cx| {
            e.perform_action(&ToggleOvertype, window, cx);
            assert!(!e.state.overtype);
            e.single_line = false;
            e.input_enabled = false;
            e.perform_action(&ToggleOvertype, window, cx);
            e.replace_and_mark_text_in_range(None, "x", None, window, cx);
            e.replace_text_in_range(None, "y", window, cx);
            assert!(!e.state.overtype);
            assert_eq!(e.text(cx), "abc");
            assert!(e.marked.is_empty());
            // Locking an already-overtype view must reject input before IME bookkeeping.
            e.state.overtype = true;
            e.sel(cx).set_selection(0, 2);
            let before = (e.doc(cx).version, e.state.selections.clone());
            e.replace_and_mark_text_in_range(None, "仮", None, window, cx);
            e.replace_text_in_range(None, "界", window, cx);
            e.perform_action(
                &crate::config::customization::InsertText { text: "bad".into(), replace: None },
                window,
                cx,
            );
            assert_eq!(e.text(cx), "abc");
            assert_eq!((e.doc(cx).version, e.state.selections.clone()), before);
            assert!(e.marked.is_empty());
        });
    }
}

#[cfg(test)]
mod unicode_literal_tests {
    use super::*;
    use crate::keybindings::{Command, CommandAction};
    use gpui::TestAppContext;

    fn command(e: &Entity<Editor>, cx: &mut gpui::VisualTestContext, name: &str, args: serde_json::Value) {
        e.update_in(cx, |e, window, cx| {
            let a = Command { command: name.into(), args }.build(cx).expect("dedicated insertion command");
            assert!(e.perform_action(a.as_ref(), window, cx));
        });
        cx.run_until_parked();
    }

    fn arm(e: &Entity<Editor>, cx: &mut gpui::VisualTestContext) {
        command(e, cx, "editor::InsertLiteralNextKey", serde_json::json!({}));
    }

    #[gpui::test]
    fn u01_scalar_commands_insert_emoji_and_combining_atomically(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("ab\ncd", None, cx);
        e.update(cx, |e, cx| e.edit(cx, |s| s.reselect(vec![Selection::caret(1), Selection::caret(4)])));
        command(&e, cx, "editor::InsertUnicodeCharacter", serde_json::json!({"value":"U+1F600"}));
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "a😀b\nc😀d");
        command(&e, cx, "editor::InsertUnicodeCharacter", serde_json::json!({"value":"0301"}));
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "a😀\u{301}b\nc😀\u{301}d");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "a😀b\nc😀d");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "ab\ncd");
    }

    #[gpui::test]
    fn u01_literal_bypasses_a_real_binding_for_one_key_and_escape_cancels(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("", None, cx);
        cx.update(|_, cx| cx.bind_keys([gpui::KeyBinding::new("x", SelectAll, Some("Editor"))]));
        arm(&e, cx);
        cx.simulate_keystrokes("x");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "x");
        cx.simulate_keystrokes("x");
        assert_eq!(e.read_with(cx, |e, _| e.state.selections.clone()), [Selection::new(0, 1)]);
        arm(&e, cx);
        cx.simulate_keystrokes("escape x");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "x");
        arm(&e, cx);
        cx.simulate_keystrokes("(");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "(", "literal text must not auto-pair");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "x");
    }

    #[gpui::test]
    fn u01_invalid_scalars_preserve_text_selection_and_history(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("a😀|b", None, cx);
        e.update(cx, |e, cx| {
            let before = (e.text(cx), e.state.selections.clone(), e.doc(cx).version, e.doc(cx).save_point());
            for value in
                ["", "U+", "U+D800", "dfff", "110000", "FFFFFFFF", "U+1F600x", "0x41", "41 42", "+41", "-1", "😀"]
            {
                assert!(e.try_insert_unicode_character(value, cx).is_err(), "{value}");
                assert_eq!((e.text(cx), e.state.selections.clone(), e.doc(cx).version, e.doc(cx).save_point()), before);
            }
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "a😀b", "no invalid request created history");
            e.try_insert_unicode_character(" u+10ffff ", cx).unwrap();
            assert_eq!(e.text(cx), "a😀\u{10ffff}b");
        });
    }

    #[gpui::test]
    fn u01_native_literal_emoji_combining_and_ime_ranges_keep_normal_commit(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("a|z", None, cx);
        arm(&e, cx);
        e.update_in(cx, |e, w, cx| e.replace_text_in_range(None, "😀", w, cx));
        arm(&e, cx);
        e.update_in(cx, |e, w, cx| {
            e.replace_text_in_range(None, "\u{301}", w, cx);
            assert_eq!(e.text(cx), "a😀\u{301}z");
            assert_eq!(e.selected_text_range(false, w, cx).unwrap().range, 4..4);
        });
        arm(&e, cx);
        e.update_in(cx, |e, w, cx| {
            e.replace_and_mark_text_in_range(None, "仮", Some(0..1), w, cx);
            assert!(e.literal_next.is_none(), "native composition takes precedence");
            let before = (e.text(cx), e.marked_text_range(w, cx), e.state.selections.clone());
            assert!(e.try_insert_unicode_character("1f600", cx).unwrap_err().contains("composition"));
            e.insert_literal_next_key(&InsertLiteralNextKey, w, cx);
            assert_eq!((e.text(cx), e.marked_text_range(w, cx), e.state.selections.clone()), before);
            e.replace_and_mark_text_in_range(None, "かな", Some(2..2), w, cx);
            e.replace_text_in_range(None, "愛", w, cx);
            assert_eq!(e.text(cx), "a😀\u{301}愛z");
            assert!(e.marked.is_empty());
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "a😀\u{301}z");
        });
    }

    #[gpui::test]
    fn u01_literal_shortcuts_navigation_blur_and_mode_changes_cancel(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("a|b", None, cx);
        cx.update(|_, cx| cx.bind_keys([gpui::KeyBinding::new("x", SelectAll, Some("Editor"))]));
        for key in ["ctrl-x", "cmd-a", "left", "escape"] {
            arm(&e, cx);
            cx.simulate_keystrokes(key);
            assert!(e.read_with(cx, |e, _| e.literal_next.is_none()), "{key}");
            assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "ab");
        }
        arm(&e, cx);
        cx.update(|w, _| w.blur());
        cx.run_until_parked();
        e.update_in(cx, |e, w, _| e.focus(w));
        cx.simulate_keystrokes("x");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "ab");
        arm(&e, cx);
        command(&e, cx, "editor::ToggleOvertype", serde_json::json!({}));
        cx.simulate_keystrokes("x");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "ab");
        arm(&e, cx);
        e.update(cx, |e, cx| e.edit(cx, |s| s.insert("!")));
        cx.simulate_keystrokes("x");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "!");
    }

    #[gpui::test]
    fn u01_literal_window_deactivation_cancels_before_returning_to_editor(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("ab", None, cx);
        cx.update(|_, cx| cx.bind_keys([gpui::KeyBinding::new("x", SelectAll, Some("Editor"))]));
        arm(&e, cx);
        cx.deactivate_window();
        cx.run_until_parked();
        assert!(e.read_with(cx, |e, _| e.literal_next.is_none()));
        cx.update(|w, _| w.activate_window());
        cx.simulate_keystrokes("x");
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "ab");
    }

    #[gpui::test]
    fn u01_option_preedit_key_char_defers_to_native_ime_before_insertion(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("a|z", None, cx);
        arm(&e, cx);
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: gpui::Keystroke {
                key: "e".into(),
                key_char: Some("´".into()),
                modifiers: gpui::Modifiers { alt: true, ..Default::default() },
            },
            is_held: false,
        });
        assert_eq!(
            e.read_with(cx, |e, cx| e.text(cx)),
            "az",
            "preedit key_char must reach native interpretation first"
        );
        assert!(e.read_with(cx, |e, _| e.literal_next.is_none()));
        e.update_in(cx, |e, w, cx| {
            e.replace_and_mark_text_in_range(None, "´", Some(0..1), w, cx);
            assert_eq!(e.marked_text_range(w, cx), Some(1..2));
            e.replace_text_in_range(None, "é", w, cx);
            assert_eq!(e.text(cx), "aéz");
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "az");
        });
    }

    #[gpui::test]
    fn u01_active_input_method_keeps_initial_native_preedit_key(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("a|z", None, cx);
        arm(&e, cx);
        // GPUI key_char can describe the fallback keyboard layout even under an input method.
        cx.update(|_, cx| cx.set_global(LiteralKeyboardInput(false)));
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: gpui::Keystroke { key: "a".into(), key_char: Some("a".into()), modifiers: Default::default() },
            is_held: false,
        });
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "az");
        assert!(e.read_with(cx, |e, _| e.literal_next.is_none()));
        e.update_in(cx, |e, w, cx| {
            e.insert_literal_next_key(&InsertLiteralNextKey, w, cx);
            assert!(e.literal_next.is_none(), "active input methods retain native interpretation");
            e.replace_and_mark_text_in_range(None, "あ", Some(1..1), w, cx);
            assert_eq!(e.marked_text_range(w, cx), Some(1..2));
            e.replace_text_in_range(None, "愛", w, cx);
            assert_eq!(e.text(cx), "a愛z");
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "az");
        });
    }

    #[gpui::test]
    fn u01_unmodified_layout_dead_key_keeps_native_preedit(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("a|z", None, cx);
        arm(&e, cx);
        cx.update(|_, cx| cx.set_global(LiteralNativeKey(false)));
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: gpui::Keystroke { key: "^".into(), key_char: Some("^".into()), modifiers: Default::default() },
            is_held: false,
        });
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "az", "native dead-key state precedes layout key_char");
        assert!(e.read_with(cx, |e, _| e.literal_next.is_none()));
        e.update_in(cx, |e, w, cx| {
            e.replace_and_mark_text_in_range(None, "^", Some(0..1), w, cx);
            assert_eq!(e.marked_text_range(w, cx), Some(1..2));
            e.replace_text_in_range(None, "ê", w, cx);
            assert_eq!(e.text(cx), "aêz");
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), "az");
        });
    }

    #[gpui::test]
    fn u01_review_native_return_and_keypad_enter_insert_plain_newline(cx: &mut TestAppContext) {
        for native in ["\r", "\u{3}"] {
            let (e, cx) = focused_editor("    a|", None, cx);
            arm(&e, cx);
            cx.update(|_, cx| cx.set_global(LiteralNativeText(Some(native.into()))));
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    key: "enter".into(),
                    key_char: Some("\n".into()),
                    modifiers: Default::default(),
                },
                is_held: false,
            });
            assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "    a\n");
            cx.simulate_keystrokes("cmd-z");
            assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "    a");
            cx.simulate_keystrokes("enter");
            assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "    a\n    ", "ordinary autoindent stays intact");
        }
    }

    #[gpui::test]
    fn u01_review_resolved_literal_macro_ignores_live_input_source(cx: &mut TestAppContext) {
        use crate::keybindings::Recorder;
        let (ws, cx) = crate::workspace::ui_tests::window(cx);
        cx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
        cx.dispatch_action(InsertLiteralNextKey);
        cx.simulate_keystrokes("(");
        let recorded = cx.update(|_, cx| {
            let r = cx.default_global::<Recorder>();
            r.recording = false;
            r.commands.clone()
        });
        assert_eq!(recorded.len(), 1, "record resolved text, not transient arming");
        assert_eq!(recorded[0].command, "editor::InsertLiteralText");
        let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        assert_eq!(editor.read_with(cx, |e, cx| e.text(cx)), "(");
        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, cx| {
            cx.set_global(LiteralKeyboardInput(false));
            cx.default_global::<Recorder>().last = recorded;
        });
        cx.dispatch_action(crate::workspace::ReplayMacro);
        assert_eq!(editor.read_with(cx, |e, cx| e.text(cx)), "(");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(editor.read_with(cx, |e, cx| e.text(cx)), "");
    }

    #[gpui::test]
    fn u01_qualified_native_text_preserves_caps_unicode_and_control_safety(cx: &mut TestAppContext) {
        for (native, expected) in [
            (Some("A"), "A"), // Caps Lock may differ from GPUI's fallback layout text.
            (Some("é"), "é"),
            (Some("😀\u{301}"), "😀\u{301}"),
            (Some("\t"), "\t"),
            (Some("\n"), "\n"),
            (Some("\u{1}"), ""),
            (Some("\r"), ""), // Unqualified controls remain rejected on an ordinary text key.
            (Some("\u{3}"), ""),
            (Some("a\u{0}b"), ""),
            (None, ""),
        ] {
            let (e, cx) = focused_editor("", None, cx);
            arm(&e, cx);
            cx.update(|_, cx| cx.set_global(LiteralNativeText(native.map(str::to_string))));
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke {
                    key: "a".into(),
                    key_char: Some("a".into()),
                    modifiers: Default::default(),
                },
                is_held: false,
            });
            assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), expected, "native event text: {native:?}");
            assert!(e.read_with(cx, |e, _| e.literal_next.is_none()));
            cx.simulate_keystrokes("cmd-z");
            assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "");
        }
    }

    #[gpui::test]
    fn u01_protected_views_refuse_scalar_and_literal_arming(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("a|b", None, cx);
        for mode in ["immutable", "viewer", "follow", "readonly", "field"] {
            e.update_in(cx, |e, w, cx| {
                e.input_enabled = mode != "immutable";
                e.single_line = mode == "field";
                e.update_doc(cx, |d| {
                    d.read_only = mode == "readonly";
                    d.set_viewer_mode(mode == "viewer");
                    d.set_following(mode == "follow");
                });
                let before = (e.text(cx), e.state.selections.clone(), e.doc(cx).version);
                assert!(e.try_insert_unicode_character("1f600", cx).is_err(), "{mode}");
                e.insert_literal_next_key(&InsertLiteralNextKey, w, cx);
                assert!(e.literal_next.is_none(), "{mode}");
                assert_eq!((e.text(cx), e.state.selections.clone(), e.doc(cx).version), before);
            });
        }
    }

    #[gpui::test]
    fn u01_late_peer_composition_refuses_literal_native_event_then_shared_undo_works(cx: &mut TestAppContext) {
        let (e, cx) = focused_editor("a|b", None, cx);
        let peer = e.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
        arm(&e, cx);
        peer.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(None, "仮", None, w, cx));
        let before =
            peer.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version, e.doc(cx).save_point()));
        e.update_in(cx, |e, w, cx| e.replace_text_in_range(None, "😀", w, cx));
        assert_eq!(
            peer.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version, e.doc(cx).save_point())),
            before
        );
        assert!(e.read_with(cx, |e, _| e.literal_next.is_none()));
        peer.update_in(cx, |e, w, cx| e.replace_text_in_range(None, "愛", w, cx));
        e.update(cx, |e, cx| {
            e.catch_up(cx);
            e.sel(cx).set_cursor(2);
        });
        command(&e, cx, "editor::InsertUnicodeCharacter", serde_json::json!({"value":"1F600"}));
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "a愛😀b");
        peer.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "a愛b");
        peer.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "ab", "refused literal event added no Undo entry");
    }
}
