# Text essentials: command line, encodings, backups and JSON

These four features were chosen because they serve text editing. Commands are available in
the menus and Command Palette. They have no new default global shortcuts.

## Command-line interface

```sh
Orlo --help
Orlo --version
Orlo --wait notes.txt
Orlo crates/app/src/main.rs:25:8
printf '{"hello":"world"}' | Orlo -
Orlo -- ./name:12 ./-literal-name
export EDITOR='Orlo --wait'
```

Put the built executable on your PATH, or replace `Orlo` with its absolute path. A running
instance receives requests through the existing local handoff. `--wait` waits for the final
view of every requested buffer to close, including shared views in other windows. A fresh
`--wait` launch starts the GUI separately, so the client can finish while unrelated windows
remain. Canceling Quit keeps the wait alive; completing Quit releases it.

Locations are one-based Unicode character positions and clamp to the available line/column.
An existing filename containing colons takes precedence over a location suffix. `--` makes
all following arguments literal paths. Missing ordinary files open as named empty buffers;
Save creates them. Explicit `-` reads valid UTF-8 into a new unsaved buffer, capped at 1 MiB.
Requests allow up to 4096 targets and 1 MiB of combined path bytes.

Help and version exit before starting the application or reading stdin. Invalid arguments
exit 2; failed delivery/open requests exit nonzero. An ordinary fresh GUI launch retains
its foreground lifetime, so an initial open error is reported when that GUI exits. Running
instance requests and fresh `--wait` requests receive confirmed open failures immediately.
There is no stdout editing, CLI search/diff or arbitrary-command interface.

## Syntax selection

**Selection > Expand Syntax Selection** grows every selection to the smallest enclosing named
node in the bundled parser's current tree. Repeat to go from an identifier to an expression,
then its statement and enclosing blocks. Equal-size wrappers are skipped. A caret uses the
character on its right; at the end of the document it uses the final character on its left.
Empty documents have no larger range. Valid named ranges remain usable in malformed source.

**Shrink Syntax Selection** reverses each expansion exactly, including reversed selections,
multiple carets and virtual endpoints even when expanding merged overlaps. Each view has its
own temporary stack. Editing, changing syntax or moving selections independently (including
moving away and back, Undo Selection or Redo Selection) ends that stack. These commands also
work in read-only, immutable and Follow views and never change text, dirty state or text undo.

Both commands are in the command palette under **Selection**. They have no default shortcut;
user keymaps can bind `editor::ExpandSyntaxSelection` and `editor::ShrinkSyntaxSelection`.
**Expand Selection** and its existing Shift-Command-A shortcut retain lexical Smart Expand.

Syntax selection needs a Tree-sitter grammar, bundled or a [language folder's](adding-a-language.md),
and a document at most 16 MiB in UTF-8.
If parsing is still pending, the command queues normal highlighting and asks you to retry;
it leaves the selections where they are, including if text or carets change while parsing.
This can also happen just after a file opens in the background; retry after preparation.
Unavailable support, no larger node and no shrink history report a status message.

## Paragraph and sentence navigation

**Edit > Prose** and the palette's **Edit** commands offer Previous/Next Paragraph and
Previous/Next Sentence, their Select Previous/Next variants, Select Sentence and Delete
Sentence. No default shortcuts are assigned. Existing Expand Selection to Paragraph keeps
its behavior.

Paragraphs are runs of nonblank physical lines; whitespace-only blank lines separate them.
A paragraph starts at its first line's beginning, including indentation. Both motion directions
visit the same ordered starts plus the document's beginning and end. From inside a paragraph,
Previous goes to its start and Next to the following start; at an exact boundary they move
strictly to the preceding/following boundary. A run of blank lines adds no intermediate stops.

Sentence navigation uses a punctuation heuristic, not linguistic analysis: `.`, `?` or `!`
ends a sentence when followed by whitespace or document end, allowing stacked closing quotes
and brackets between the punctuation and whitespace. ASCII quotes and Unicode closing
punctuation are supported, for example `“Really?!”)]  Next`. Abbreviations have no special
exception: `Dr. Li` is two sentences under this rule, while an internal period in `e.g.`
does not split unless it is followed by a separator. Blank lines without terminal punctuation
do not end a sentence. A final unpunctuated remainder is one sentence.

Sentence motions visit sentence starts plus document beginning/end, using the same strict
boundary rule in both directions. Separator whitespace belongs to the preceding sentence;
initial whitespace belongs to the first sentence. Select Sentence selects the containing
interval, including following spaces and line breaks. At a start it chooses that sentence,
in separator whitespace the preceding one, and at EOF the last one. Existing selections
expand to every touched sentence; an end exactly at the next start excludes that next sentence.
Backward selections retain direction. Repeating Select Sentence is idempotent.

Delete Sentence deletes those same intervals. Overlapping requests from multiple carets
merge into one undoable edit; Undo restores the original selections and Redo repeats it.
Empty documents and purely virtual selections delete nothing. Virtual columns never add
padding; physical selections with virtual endpoints use their physical text. A previous motion
from virtual space first returns to the real line end, following ordinary word motion. Motions and
selection work in read-only viewers and Follow mode, without editing text or text history.
Delete follows ordinary source-edit admission, including protected Follow/viewer refusal and
working-copy edits to filesystem-read-only files. Unicode graphemes (combining marks and
emoji sequences) and CRLF pairs stay whole. Wrapped rows and folds use the existing display
navigation behavior while boundaries remain source positions.

Custom bindings and macros use `editor::Move` with `motion` set to `paragraph_previous`,
`paragraph_next`, `sentence_previous` or `sentence_next`; `select: true` extends each existing
anchor. The other actions are `editor::SelectSentence` and `editor::DeleteSentence`.
For example:

```json
{"keys":["cmd-alt-j"],"command":"editor::Move","args":{"motion":"sentence_next","select":true},"context":"Editor"}
```

## Transpose words

**Edit > Text > Transpose Words** swaps the word at each caret with the following word.
At a word's end or in separating punctuation or whitespace, it swaps the words on either side;
at the exact document end it swaps the final two. For example, `alpha, beta` becomes
`beta, alpha`, retaining the comma and space. Tabs, line breaks and emoji separators remain
exactly as written, including combining marks attached to separators. Words use Orlo's existing
Unicode letter/number/underscore classification,
including attached combining marks; grapheme clusters stay whole.

A selection uses its head. A missing partner, trailing whitespace before EOF, a virtual head,
or identical words changes nothing. If any multicaret swap overlaps another or shares a word,
the entire command does nothing. Disjoint pairs form one Undo step that restores all original
selections; Redo repeats the swaps. Selections follow the same change mapping as other edits, including its handling of unchanged
common text and insertion boundaries.
No virtual padding is inserted. Source coordinates apply across wrapping and folds.

The palette entry is **Edit: Transpose Words**. It has no default shortcut; custom bindings
and macros use `editor::TransposeWords`. Existing **Transpose** (`ctrl-t`) keeps its grapheme
swap and selected-text rotation behavior.

## File encodings

New and unmarked files default to UTF-8. Unicode BOMs are detected automatically, including
UTF-32 before UTF-16. Orlo does not guess legacy encodings. Use **File > Reopen with Encoding**
(or the command palette) to interpret an unmarked file with a named codec. Reopen reads the
file without writing it and refuses unsaved edits or a save in progress. Malformed input opens
read-only and cannot overwrite its original, including through a symbolic or hard link.

The exact supported choices are:

- UTF-8, UTF-16 LE, UTF-16 BE, UTF-32 LE, UTF-32 BE.
- Windows-1250, Windows-1251, Windows-1252, Windows-1253, Windows-1254, Windows-1255,
  Windows-1256, Windows-1257, Windows-1258.
- ISO-8859-1, ISO-8859-2, ISO-8859-5, ISO-8859-7, ISO-8859-15.
- Shift JIS, EUC-JP, ISO-2022-JP, GBK, GB18030, Big5, EUC-KR.

ISO-8859-1 is true Latin-1: bytes 80–9F are C1 controls, unlike Windows-1252 punctuation.
Other legacy codecs follow the WHATWG Encoding Standard mappings supplied by encoding_rs:
Shift JIS includes Windows extensions, EUC-KR uses the extended Windows-949 repertoire, and
Big5 includes WHATWG extensions. These labels do not promise every similarly named historical
vendor variant. No Unicode normalization or Vietnamese tone-mark decomposition is performed.
Equivalent legacy byte sequences may be canonicalized when saved; decoded text must stay identical.

**File > Save with Encoding** changes the output codec and runs ordinary Save, including
format-on-save. Each Unicode encoding offers both BOM and no-BOM output; legacy encodings have
no BOM. Existing newline policy is preserved. An unrepresentable character, a codec mapping
that changes text on reopening (such as the Shift JIS yen/backslash alias), or leading bytes
that would be mistaken for a different BOM produces an error before any write or history
capture. Conversion errors identify the first affected Unicode scalar (character and U+ code
point), with a one-based line and column. Columns count Unicode scalars, including each tab
and combining mark, rather than bytes or display cells. LF, CRLF and lone CR each end one
diagnostic line in the text being encoded. Choose a Unicode codec or edit the affected text
and save again. Save As, Save a Copy As, automatic saves, Statistics and checksums report
these conversion details too. A failed or canceled save retains the unsaved format choice,
and undoing text does not discard it. The status bar shows the requested output encoding.

Shared views, session/workspace recovery, recent closed-file reopening and historical snapshots
retain the named disk interpretation. It remains separate from a pending save conversion.
An external Unicode BOM overrides remembered hints on ordinary reload; explicit Reopen with
Encoding against a conflicting BOM stays protected. Hints guide decoding only and do not restore
old indentation or display settings.

Find in Files with **Use Buffer** searches the interpreted live text. Disk search, replacement,
extraction, file/folder comparison and closed-file pipelines use available open-document or recent
closed-file hints; otherwise they use BOM detection then UTF-8. Unknown non-UTF-8 or malformed
files appear as errors in disk search with a request to reopen with the correct encoding;
substitution text is not searched or replaced. Closed pipelines skip them and comparison
reports them as unsupported text. Open and reinterpret them first for exact
text operations. Lexical **Go to Reference** applies the same policy to closed files, using
retained disk-codec hints and reporting unsupported files with their paths. It searches open
buffers' current text directly. Replacements preserve the preview's codec and refuse stale bytes or conversion
errors. Git patch, partial staging and blame retain their existing explicit UTF-8/binary limits;
they do not transcode repository blobs or patch coordinates.

## Historical backups

Set `"file_history": true` in Preferences > Settings to keep original disk bytes before user
saves and destructive closed-file tools. The default is false. History is separate from
session recovery, stored privately in `~/Library/Application Support/Orlo/File History`.
An enabled capture failure stops the overwrite and reports an error.

File > File History > Browse Versions lists saved versions and opens a read-only preview. Restore
Previewed Version (Command-Enter while previewing) restores its text into the active buffer
as one undoable, unsaved edit. It keeps that buffer's current output encoding, BOM and line
endings. Escape closes the preview. Clear File History requires confirmation and removes
only the active file's retained versions.

Identical versions are skipped. Retention is at most 20 versions per path and 200 MiB total;
individual source files are capped at 100 MiB. History records preserve raw bytes and original
encoding metadata, including explicitly reopened BOM-less Unicode and legacy files. Files above the size limit
cannot be overwritten while history is enabled. History does not schedule saves and uses no cloud sync.

## Save a copy

To save the current file after a formatter error, choose **File > Save Without Formatting**
or the matching command-palette entry. This is an ordinary save with cleanup and file protection;
it updates the document's saved point. It includes a native input preview just as Save does,
without committing or canceling composition.

**File > Save a Copy As…** (also **File: Save a Copy As…** in the command palette)
writes the current buffer to a chosen destination. It works for dirty named files,
untitled buffers and read-only decoded files. The source keeps its path, saved point,
unsaved status, undo/redo history, read-only flag, shared views and project association.
Explorer **Duplicate…** still copies the file's bytes on disk.

The snapshot is taken after you choose the destination, before waiting for other writes.
It uses the current text, output encoding, BOM and line ending; no formatting or save-time
cleanup runs. Edits or format changes after capture stay in the source. Like explicit
Save, a native input preview currently in the buffer is included without committing or
canceling composition. Stop Follow Log before saving a copy.

The native save panel confirms overwrites. A copy refuses the source itself (including
symlinks and hard links), any destination owned by another live document, a read-only
destination, or one changed while waiting. Choose again after such a refusal. Encoding
must represent the text without loss; choose a Unicode encoding when a legacy codec
cannot represent it. Canceling or failing leaves the source unchanged. Accepted writes
continue if their source tab closes, and Quit waits for them. Opening the destination
or a hardlink alias waits for publication and reads the resulting bytes; recovered unsaved
text stays intact while waiting.

## Automatic file saves

Open **Settings > Settings** and add `"file_autosave": true` to enable automatic saves to existing
files. It defaults to false. Global, project, language, scope and path overrides work as for
other file preferences. `"file_autosave_delay_ms": 2000` sets the idle delay in milliseconds;
Orlo waits two seconds by default after edits settle, then uses ordinary Save's format-on-save
and cleanup settings. Each edit restarts that interval. The delay accepts whole numbers from
1 through 4294967295 (including JSON forms such as `2000.0`); zero, negatives, fractions and
non-number values are rejected with a settings warning. An invalid global value keeps 2000;
an invalid project value keeps the inherited delay. An invalid scoped rule is ignored, keeping
its inherited preferences. Changing the delay or owning view restarts a full interval. Untitled, never-created, deleted, read-only, scratch and
Follow documents are skipped. Session recovery still runs independently; `file_history` remains
a separate opt-in setting and keeps originals before automatic saves too.

Shared tabs/windows have one timer. The oldest visible editor view supplies preferences and
formatting; focus does not change that owner. Closing it transfers ownership to the oldest
remaining view. Automatic work waits while any shared view has active input-method composition
or snippet fields, a Save As chooser is open for the document, or a save/write is in progress.
Finishing composition or snippet navigation resumes the timer without requiring more typing.

A changed external file is protected: automatic saving pauses with a status explanation and
never opens Save As or overwrites a known changed file. Reload or save explicitly to resolve it.
A conversion/write failure leaves the document dirty and pauses that text/output format;
a new edit or changed save policy can retry, including changing only the codec, BOM or line
ending. Formatting's own edit does not start an error/retry loop. Restored dirty documents
and moved paths with no trusted disk baseline require an explicit Save or reload first.

Disabling the setting, discarding a tab, or quitting cancels pending work. Close prompts suspend
it until a choice is made. A write already captured completes through the normal save/quit
waiters; edits made meanwhile remain unsaved and get another timer. Disk checks run before
history capture and immediately before safe publication or in-place writes. These checks cannot
lock other applications out of the filesystem; an external writer racing the final check can
still change a file. A write receipt uses the actual written file's identity, so a replacement
noticed at completion is never trusted as the next autosave baseline.

## JSON tools

Edit > JSON offers Format, Minify and Validate. Each nonempty selection is a separate JSON
value; with only carets, the whole document is used. Format uses two spaces, compact empty
containers and no added final newline. Numeric spelling, key order, duplicate keys, string
escapes and Unicode are preserved; only whitespace outside strings changes.

Every target must validate before an edit applies. Errors report line/column and the target's
starting document line. Rewrites form one undo step and update shared views. Validate changes
no text or undo history and works in read-only buffers. Background results are discarded if
the target, selections, text or latest JSON command changes.

Limits are 16 MiB total selected UTF-8 input, 32 MiB output and 4096 selections. Formatting and
large requests run off the UI thread. There is no JSON tree browser, schema engine or expanded
XML/SQL tool suite.

## Literal escaping

Select text and use **Edit > Convert** or the matching **Convert:** palette commands.
Each nonempty physical selection is independent; carets and selections entirely beyond line
ends are skipped. With no selected source text these commands do nothing. Mixed selections
never receive virtual padding or extra quotes at empty carets. All targets must succeed before
one Undo step applies; a failure reports its starting document line and keeps every target,
selection and undo/redo history unchanged.

- **Escape Regex** escapes Orlo regex metacharacters using its installed regex syntax:
  `\ . + * ? ( ) | [ ] { } ^ $ # & - ~`. The result matches the original literal text
  in ordinary case-sensitive regex mode. It does not add anchors or change Find options.
- **Quote JSON String** writes one JSON string, including its surrounding double quotes.
  **Unquote JSON String** accepts exactly one JSON string, with optional surrounding JSON
  whitespace. JSON numbers, objects, arrays, booleans and null are errors. Quotes, backslashes,
  newlines and emoji round-trip through the existing JSON parser.
- **Decode Text Escapes** decodes exactly `\\`, `\"`, `\'`, `\n`, `\r`, `\t`, `\b`, `\f`,
  `\xHH`, `\uHHHH` and `\UHHHHHHHH` once. Hex digits are ASCII, case-insensitive and fixed
  width. `\xHH` means U+00HH (`\xFF` is `ÿ`; `\xC3\xA9` is `Ã©`, not `é`).
  Adjacent high/low `\u` surrogate escapes combine, so `\uD83D\uDE00` becomes `😀`.
  Isolated surrogates, non-scalar values, unknown escapes, incomplete escapes and a trailing
  backslash reject all targets. Unescaped Unicode and control characters stay literal;
  this is not a language interpreter or a regex replacement template.

These explicit commands preserve the existing editor newline policy: inserted CRLF pairs
become logical LF; output file encoding, BOM and line-ending choices remain unchanged.
Successful explicit commands end marked input like other editing commands; failed preflight
retains it. Immutable viewers and Follow do not edit. Read-only source-file protection still
allows buffer editing and prevents unsafe overwrite on Save.

There are no default shortcuts or preferences. Custom keymaps/macros can use
`editor::Convert` with `{"codec":"regex_literal","decode":false}`,
`{"codec":"json_string","decode":false}` or `{"codec":"json_string","decode":true}`;
`editor::DecodeTextEscapes` needs no arguments. Regex Literal only supports escaping;
requesting its decode direction reports an error. URI Component and HTML Entity commands
keep their separate existing dialects.

Text Pipelines use `{"op":"encode","codec":"regex_literal"}`,
`{"op":"encode","codec":"json_string"}`, `{"op":"decode","codec":"json_string"}` or
`{"op":"decode_text_escapes"}`. These steps preserve the existing pipeline target policy:
a selection stays a text selection without line widening; with no selection the pipeline
runs on the document, and file runs process each file. The pipeline's normal CRLF-to-LF
policy still applies between steps. A loaded Regex Literal decode step is rejected before
the pipeline runs.

## Line numbers

**View > Line Numbers > Relative** displays distances from the primary caret's physical source
line; that line keeps its absolute number. **Absolute** restores the default labels. Both are
available in the command palette under **View: Line Numbers**. Distances count hidden folded
lines, and each shared view uses its own primary caret; secondary carets do not change the
origin. Wrapped continuation rows and comparison filler rows have no number. A fold joining
several source lines keeps the first source line's label, even when the caret is in its tail.
Go to Line always uses absolute source line numbers. Numbering changes no text, selections,
Undo history or mouse targets, and respects distraction-free gutter visibility.

## Code folding

Use **Edit > Code Folding > Fold**, **Fold Level**, or the triangles beside line numbers.
With a current bundled parser, Orlo finds multiline brace/bracket blocks, block comments,
JSX/TSX elements, and Python, Ruby and shell blocks from syntax. Indentation is not required
for brace/comment/markup folds. The header stays visible; folding hides through the closing
delimiter, while code following that delimiter stays visible. Nested headers fold independently.
Fold Level 1 selects the outermost syntax regions, Level 2 their children, and so on.
Incomplete or malformed code uses the parser's recovered structure: complete neighboring blocks
remain foldable, and any recovered fold stays within the current text.

Unsupported syntaxes (including standalone HTML), installed Sublime grammars, files beyond the
parser's 16 MiB limit, and text still waiting for parsing use indentation folding. In that
fallback, Fold Level follows indentation levels. Commands act on the discovery currently
available; they do not wait for parsing. After an edit, stale syntax ranges are never reused.

A selection within one line still folds exactly the selected characters. A multiline selection
folds the selected lines after its first, as before. Click a placeholder or use **Unfold** to
open a fold; **Unfold All** opens everything. Folds belong to each view and work in read-only
files. They change neither saved text nor undo history; edits inside a fold reveal its text.

Collapsed folds are remembered separately for each view in sessions and project workspaces,
including nested folds and exact inline selections. They return only when the entire decoded
text still matches. A changed file, damaged fold record or older session opens expanded;
fold restoration never changes file bytes or undo history. Invalid fold metadata leaves
otherwise recoverable unsaved text and the other views' valid folds intact. During native
text composition, recovery excludes the provisional text and omits folds tied to that live
preview; those recovered views open expanded.

Closing a named view remembers its folds. **Reopen Closed File** brings back that view with
its own folds, even when another view of the file remains open. Ordinary Open or Recent uses
the most recently closed view's folds when creating a tab; focusing an existing tab keeps its
current folds. The latest 32 closed views retain fold metadata across launches in that window.
Discarded text is not kept by this history. Folding still works in larger documents, but fold
persistence is limited to 4096 collapsed ranges per view and 16 MiB of decoded UTF-8 text.


## Overtype

Choose **Edit > Toggle Overtype**, search for **Edit: Toggle Overtype** in the Command Palette,
or click **INS** / **OVR** in the status bar. **INS** inserts text; **OVR** replaces the graphemes
ahead of each caret. Replacements stop before the line ending and extend the line as needed.
Combining marks and emoji continuations that extend the preceding grapheme keep the following
text. Selected text is replaced exactly. Overtype disables automatic bracket/quote pairing and
step-over. Paste, Tab, newline, snippets and explicit input-method replacement ranges keep their
usual behavior. A text input chunk containing a tab or line break inserts as a whole. Undo and
Redo restore text without changing the mode.

Each view starts in insertion mode, including a new view of an already-open file and views
restored after reopening Orlo. The mode is not saved in preferences or sessions. It is unavailable
in palette/find/form fields and immutable viewers. Switching tabs or windows shows the active
source view’s mode; typing in Find/Replace fields always uses their independent insertion mode.
A read-only file can still be edited as a working copy, including overtype and IME commits,
but its original stays protected from Save. Follow locks source editing in every shared view.
The action name for custom key bindings is
`editor::ToggleOvertype`; it has no default shortcut.

During IME composition in overtype, marked preview text appears before the original target.
Updates and cancellation preserve that original text; committing replaces it once. Canceling
removes only the preview and restores the original selections, including their direction. If
another view edits the document, or a selection/motion command ends
composition, the preview stays as inserted text and its old replacement target is discarded.

## Virtual space

Set `"virtual_space": "rectangular"` in Preferences → Settings to place Option-drag rectangles
and Add Caret Above/Below carets beyond line ends. Use `"all"` to also enable ordinary Right,
clicks and drags there. The default is `"off"`; legacy `false` means off and `true` means all.
The preference also works in project and language-scoped settings. Editable file views use it;
single-line fields and immutable viewers stay clamped to their text. Rectangular origin stays
with each caret through motion, typing, snippets, completion and undo. Snippet fields and mirrors
retain their expansion's origin through Tab/Shift-Tab. An ordinary click or source-range selection
creates an ordinary selection.
Switching to rectangular clamps ordinary gaps; switching off clamps all gaps without editing text.
A merged selection with both ordinary and rectangular origins counts as ordinary.

With all mode, Right, clicks and drags can place a caret after the last character. Up/Down/Page keep the
caret's measured horizontal position on shorter lines. Shift extends selections there, and
Option-drag rectangles and multiple carets keep their separate positions. Virtual columns are
one drawn space wide after the shaped text, including tabs and Unicode. They begin only on
the final row of a logical line; soft-wrap continuations, fold markers and filler rows retain
their normal navigation. Home/End and document navigation return to real text. Word Left from
a gap reaches its line end before moving into the preceding word.

Moving there changes no file bytes. Typing, paste, Tab, Enter, column insertion and explicit
snippets materialize padding only when text needs that position. Padding follows the current
indentation style: spaces, or tabs to tab stops plus any remaining spaces. Existing explicit
spaces and tabs stay intact. Carets sharing a line share padding; tabs never jump over another
caret's target. When folds or wrapping give the view different tab stops from the physical
line, padding uses spaces to keep the displayed target; native replay in such views also uses
spaces conservatively. Explicit Pad to Column and Column Editor short-line padding still insert spaces. Padding and the edit undo together, restoring the virtual selections; overtype uses
the padded insertion position. Backspace crosses a virtual gap without deleting text, Delete
there does nothing, and word-backspace reaches the real line end. Deleting a virtual selection
removes only its real selected text. Copy omits imaginary spaces; a purely virtual caret or
selection copies/cuts nothing. Whole-line clipboard entries still paste above empty carets’ lines; over a selection they replace
at its start, including a virtual start.

Completion and snippet triggers do not read the word behind a virtual gap. Explicit snippets
can insert there. IME previews temporarily insert the needed padding; cancelling removes both
preview and padding and restores selection direction and the primary caret. Moving, changing
the setting or editing from another view ends that preview as ordinary text, as with other
compositions. Later input never removes source using stale preview positions.

Disabling the setting clears virtual positions without inserting spaces. Live undo and selection
history retain them while enabled, but session restore and reopening keep only real text offsets.
Saving during virtual navigation writes the unchanged source; normal save cleanup settings still
apply to spaces you have actually inserted.

## Printing and PDF

Choose **File > Print…**, or **File: Print…** in the command palette. Orlo captures the
current source, including folded text, when you open the options. Later edits do not alter
that print job. Choose **Whole document** or **Selected text**; selection printing includes
all nonempty selections in source order, merging overlaps and separating excerpts with a
newline. It does not fall back to printing the document when nothing is selected.

The options start at 11-point system monospaced text, tab width 4, and no line numbers.
Choose a font size from 8 to 24, a tab width from 1 to 16, and optional source line numbers.
Wrapping is on and syntax colors are off by default. Turn wrapping off to fit the longest
line to the page width; very long lines therefore make the print smaller. Syntax colors use
the captured syntax choice and a darker palette for white paper. If classification is unavailable,
or the document exceeds 2 MB or 100,000 lines, output falls back to plain text. macOS supplies
font fallback for Unicode. Documents over 16 MiB or expanded output over 64 MiB cannot be
prepared for printing. These options apply to the current job and reset the next time you open Print.

Select **Page setup before printing** to choose paper, orientation and scaling before layout.
Canceling page setup cancels the job. Press Enter to open the macOS print panel. It provides page ranges and printer settings;
use its **PDF** button to save a paginated PDF without a printer. Escape closes Orlo's
options, and **Cancel** closes the native panel. Printing works with read-only documents
and shared views without changing text, encoding, undo history, folds or selections.
Switching or closing the original tab before submitting options cancels the request.
Finish or cancel any active input-method composition in a shared view before opening Print;
if a shared edit is still updating selections, retry after that update finishes.

The existing **Cmd-P** Goto Anything and **Cmd-Shift-P** command palette shortcuts keep their
original behavior. Print has no default shortcut; it can be bound as the `workspace::Print`
action through the normal keymap customization.

## Follow a log

Choose **File > Follow Log** (also in the command palette) for a saved, clean file. This mode
is off by default, belongs to the shared document across windows, and is not restored on
reopen. Finish pending saves/writes and any IME composition in shared views before starting.
The persistent **FOLLOW · Stop** status control or **File > Stop Following Log** stops every
view of the document. Save As is unavailable until you stop.

Following locks source edits, paste, overtype, IME, snippets, transformations, server edits,
undo/redo and saving. You can still navigate, select, copy, print and send a selection to a
terminal. Incoming changed text clears Undo history; starting and stopping without changed
text preserves it. After stopping, normal edits and undo work again, subject to the file's
source overwrite protection.

The initiating view goes to EOF. A single caret there follows appended output; selecting text
or scrolling away lets you inspect older output. Shared views have independent positions.
Choose Follow Log again or navigate to EOF to follow the tail in that view. Truncation and
rotation replace the shown snapshot, mapping selections safely; a missing path keeps the last
snapshot and waits for the replacement.

Snapshots are read and compared in the background every 500 ms, up to 100 MiB. The actual disk
encoding is used, with a new BOM taking precedence. Incomplete encoded characters/escape
sequences are withheld, including Unicode and CJK codecs; they never become replacement
characters. Malformed input, unreadable files and the size limit pause updates with an
explanation in the status tooltip. Stop, then reload or reopen with the correct encoding.
Stopping while a character/BOM is incomplete keeps the original protected against overwrite
until ordinary reload resolves the full file. Closing the last view, changing projects so no
view remains, or renaming the document cancels its worker; reopening starts normally.


## Restore discarded untitled drafts

When you explicitly choose **Don't Save** for a dirty untitled document, or **Discard All**
when closing a window, Orlo keeps a private rescue copy. Closing one view while another view
of the same document remains open does not create a copy. Named files, tool previews and
clean untitled tabs are excluded. Ordinary Quit continues to use session recovery.

Choose **File > Discarded Untitled > Restore Discarded Untitled…**, or find
**File: Restore Discarded Untitled…** in the Command Palette. The picker lists discard times
and sizes, newest first. Search matches capture time, size and the first **1 KiB** of draft text;
these bounded excerpts are loaded once when the picker opens. Selecting a draft opens its full
read-only preview. Choose **Restore as New Untitled** (Cmd-Enter) for a new, editable, unsaved
untitled document, then Save to give it a filename. Restoring keeps the rescue copy until expiry,
Delete or Clear. **Delete…** (Cmd-Backspace in the preview) permanently deletes only that copy
after confirmation. A damaged text payload reports its error and still offers Delete; valid
siblings remain available.

Rescue is on by default and keeps at most **50 drafts**, **1 MiB per draft**, and **8 MiB total**
for **30 days**. The combined limits can retain fewer than 50 large drafts. Older copies are
removed when you next discard, browse or restore. If an external change enlarges a stored file
past the per-draft limit, its error copy can remain for Delete without displacing valid drafts.
Valid drafts take priority within 50 total rows; excess damaged copies are removed. Such damaged
files can exceed the usable 8 MiB total on disk until Delete or expiry.
Whole Unicode text is kept without truncation. Copies are private local text files in
`~/Library/Application Support/Orlo/DiscardedUntitled/`.

If a copy cannot be kept, a draft is too large, or text composition is still active, Orlo
lets you **Cancel** the close or choose **Discard Without Rescue**. Finish or cancel IME
composition first to retain only the final text. If a draft or the window's tabs change during
this extra prompt, closing is cancelled so you can review them before discarding again.

**File > Discarded Untitled > Clear Discard History…** removes these rescue copies after
confirmation. It leaves open documents, session recovery and saved-file history alone.
Clear invalidates older picker choices and pending capture tickets. Delete and Clear prevent
pending preview/restore results and older actions, including those in another window, from
restoring removed text. An already open read-only preview in another window may remain visible;
its Restore and Delete actions recheck storage and fail if its copy was removed. Failed reads,
expired copies and failed deletes/clears are reported; they are never presented as a successful restore or
clear. Failed loading keeps a focused error picker even when the status bar is hidden. Typed
text is retained; clear the filter to show **Browse Rescue Again** and **Return to Document**,
or press Escape. Clear completion preserves a
replacement picker or a newly captured preview. Bounded rescue writes finish before the tabs close, so a slow disk can delay closing.

Periodic recovery snapshots leave ongoing IME input alone. A valid overtype or virtual-space
preview and its temporary padding are excluded from that snapshot, which recovers the source
before composition. Ordinary inserted previews retain their existing insertion semantics.
Closing or switching projects and quitting commit active native composition before capturing
recovery or removing views. Moving a view retains its active composition. Explicit Save behavior
is unchanged; finish or cancel composition when you want that choice reflected on disk.

Replace in Files also respects Follow across windows and file aliases. Apply rechecks the
current mode, so starting Follow after preview still skips the file with a Stop Following Log
instruction. Stop following and refresh the preview before applying normal replacements.

### Language-aware indentation

Enter uses the active syntax for Python, Rust, JavaScript and TypeScript. Block-ending
Python colons and backslash continuations add one indent, and unmatched open brackets
add an indent per opener; Python `return`, `raise`, `break`, `continue` and `pass` reduce the next line by one indent.
Indent width, tab width and tabs-versus-spaces follow the resolved document settings.
Enter keeps its normal single Undo step and splits an empty brace pair as before.
Other syntaxes retain the existing newline behavior.

Edit > Line > Reindent Selection (also in the command palette and available as
`editor::ReindentSelection` in a custom keymap) adjusts leading whitespace on covered
lines in those four syntaxes. Carets cover their own lines; a selection ending at the
start of a line excludes that last line. Overlapping selections are combined, and one
Undo restores the operation. It does not format expressions or change other text.
It uses a bounded indentation heuristic, not a full formatter: braces and continuations
establish nesting, and Python `elif`, `else`, `except` and `finally` align with a
preceding matching header, preserving an already valid branch alignment. Strings and comments do not create blocks; lines inside multiline strings or block
comments retain their whitespace. Ambiguous Python dedents without a keyword require
manual adjustment; code inside template interpolations is treated as string content.
JavaScript and TypeScript regular-expression literals are recognized in common expression
contexts. Ambiguous slash syntax declines reindent and uses the ordinary Enter rule.
To establish multiline context the command examines up to 65,536
characters from the beginning of the document; beyond that limit reindent declines and
Enter uses the ordinary newline rule. Unsupported syntaxes also decline reindent.

### Send editor selections to the terminal

Choose **Terminal > Send Selection to Terminal** or **Terminal: Send Selection** in the palette to
paste into the selected terminal tab without adding a newline. **Send and Run Selection in
Terminal** / **Terminal: Send and Run** adds one Enter after the paste. Multiple nonempty physical
selections join in document order with one newline; empty carets and virtual-only ranges are
skipped. The commands never send the whole document as a fallback or add virtual-space padding.

Both use the terminal's ordinary paste rules. With bracketed paste enabled by the target, the
text stays inside its paste frame and Run's extra Enter follows the closing frame. Otherwise,
embedded line breaks become Enter bytes and can execute commands. A selected trailing newline
is preserved by that paste protocol, and Run still adds one explicit Enter. Input is limited to
1 MiB including framing and Run's Enter; an oversized send writes nothing.

The chosen session is shown if hidden. With none, the first session uses your current default
profile and launch preferences. An exited or closed session reports an error without silently
starting a replacement. Only accepted input returns retained output review to live output;
empty, oversized, composing, stale or dead-session requests preserve the review.

Sending works from read-only shared source views and keeps source text, selection metadata,
Undo/Redo, saved identity and dirty state. Finish or cancel composition in every shared source
view first. If a shared edit is still updating the view, retry after it updates. Custom bindings
keep the action IDs `terminal::SendSelectionToTerminal` and `terminal::ExecuteSelectionInTerminal`
(Send and Run).

### Sync the terminal folder

Choose **Terminal > Sync Terminal Folder**, **Terminal: Sync Folder** in the command palette,
or **Sync Folder** in the terminal header to change the selected existing terminal to the
active file’s parent folder. An untitled file uses the workspace’s first project root.
Switching documents never changes terminal folders automatically. A custom key binding can use
`terminal::SyncTerminalFolder`; no shortcut is assigned by default.

Enable [shell integration](customization.md#terminal-launch-preferences) before launching a
supported interactive zsh session. Sync requires a fresh empty shell prompt and refuses typed
input, text composition, foreground applications/jobs and closed or exited sessions. Any input
turns readiness off; submit a command and wait for a fresh prompt before trying again. The shell
also checks its live input buffer immediately before accepting the directory command. Spaces,
quotes, Unicode and newlines in folder names are passed literally. The terminal displays a
waiting status until the shell reports its directory. New input cancels that pending confirmation;
a failed change leaves the old directory
and reports an error.

### Find in terminal output

The terminal reserves one 28px output toolbar for live status, Find, retained output and
command controls, so those state changes do not resize the output grid.

With the terminal focused, press **⌘F** or choose **Find Terminal Output** to search retained
output without running a command. Find starts with literal, case-sensitive matching. **Aa**
(Match case, ⌘⌥C in the Find field) toggles case sensitivity; **Regex** (`.*`, ⌘⌥R) enables regular expressions.
Enter/Shift-Enter and ⌘G/⇧⌘G move forward/backward, wrapping at the ends and scrolling to the
highlighted match. Copy with the terminal focused copies the actual match text, or your dragged
selection. The Find field keeps its ordinary text-editing and clipboard shortcuts.
The compact ↑/↓ and × buttons navigate and close Find. Hover the count or status to read
its full text when the toolbar truncates it in a narrow view.

Search joins soft-wrapped rows into logical lines and matches their visible Unicode text.
Regex uses Rust's linear regular-expression syntax; lookaround and backreferences are unsupported.
Empty queries and zero-width matches produce no highlights. Invalid patterns show an error.
Patterns are limited to 8 KiB, a logical line to 8 MiB, and results to 100,000 matches; narrow
the query if a limit is reached. Matches do not cross hard line breaks.

The shell keeps running while Find shows an independent retained snapshot. Output arriving
during an unpaused search is picked up by the next refresh; a completed snapshot can briefly
include a match already evicted from live scrollback. Navigating matches, scrolling, or selecting
output pauses updates on that independent snapshot, preserving its match, viewport and selection.
The count/status shows **Paused**. Click it or press **⌘⌥U** in the Find field to update from
current live output with the same query/options; this explicitly clears the old selection and
keeps Find focused. New queries/options search the displayed snapshot while paused. Query edits,
debounce, scanning and invalid-pattern errors retain the displayed cells and selection, so Close
can preserve them throughout. The shell continues running; its final output is available on Update
even after it exits. Its cells and copied text remain those of the displayed snapshot.
Closing Find preserves the displayed output, scroll and selection, with a **Retained
output** label and **Live Output** button. Typing, pasting, or choosing Live Output returns to
the live terminal. Mouse dragging in retained output selects locally, even if a running app
requested mouse reporting. Opening Find again starts from current live output. Resize, terminal
reset, and alternate-screen transitions discard incompatible retained views and search results;
stale jobs cannot revive their old cell positions. Find actions never send input to the shell.

### Review command output

With [shell integration](customization.md#terminal-launch-preferences) explicitly enabled for
new zsh sessions, Orlo recognizes OSC133 prompt, input, output and completion markers. Your own
supported markers work with Orlo's injection off too. Ordinary prompt-looking text creates no
command records. Exit status is shown when supplied; absent or malformed status is unknown.
Interrupted commands keep their output and reported status, including 130 for zsh Ctrl-C.

Use **Terminal: Previous Command** and **Terminal: Next Command** in the palette, the matching
Tools commands, or **Previous** and **Next** in the terminal toolbar. Previous starts at the
latest completed command; Next after the latest returns to live output. **Live Output** returns
directly. Review shows captured output without the prompt or command input. **Copy Output** /
**Terminal: Copy Command Output** copies the reviewed command, or the latest completed output
when live. **Terminal: Copy Last Output** (Terminal > Copy Last Terminal Output) always copies the
newest completed output, even while reviewing an older command. These actions only navigate or
copy to the clipboard; they never send text to the shell or execute output. With no command
records, they leave the terminal and clipboard alone. Custom bindings can use
`terminal::PreviousTerminalCommand`, `NextTerminalCommand`, `CopyTerminalCommandOutput`,
`CopyLastTerminalOutput` and `TerminalLiveOutput`.

Review retains at most 32 completed commands and 1 MiB of combined input/output text, with each
capture also limited by that session's scrollback capacity. Old commands and old output are
evicted; reviewing an already displayed snapshot keeps that snapshot. Clear-screen sequences
affect the current captured screen. A terminal reset abandons unfinished capture while bounded
completed review remains available. Reducing scrollback explicitly discards command archives
and unfinished capture; changing capacity also closes obsolete review views. Hiding a terminal
keeps its shell and history. Closing it signals asynchronous shell cleanup; the private launch
files are removed after the child is reaped. A shell that exits normally also releases those
files while its retained output remains available.

### Open links in terminal output

**Cmd-click** an HTTP(S) URL to open it with the system's URL handler, or an existing file
location to open it in Orlo. File locations accept `src/main.rs:12:3`, `src/main.rs(12,3)`
and compiler-style `src/main.rs:12:3:`. Explicit locations also support extensionless files
(`Makefile:12:3`); use `./Makefile` for one without a location. Quote a path containing spaces, such as
`"src/my file.rs":2:3` or `"src/my file.rs:2:3"`. Line and column numbers start at 1;
columns count source characters, including Unicode. Omitted locations start at 1, and
positions beyond the file clamp to its text. A missing file shows a status message and
does not create a new document.

Relative paths resolve against the directory recorded when their output was printed, using
the shell's local OSC7 directory updates. Later `cd` commands do not change old links.
Paused Find, retained output after Close, and command review use the displayed output's
recorded directory. Soft-wrapped paths can be activated on either visible part. Absolute
paths, `~/` paths and local `file://` URLs are also supported; remote file hosts, malformed
file URLs and other URI schemes are refused. Bare words are not detected as paths.

Ordinary dragging selects output; use **Shift-drag** when a terminal application requests
mouse events. Cmd-click activates a link without sending it or mouse reports to the shell.
If output has mixed or unknown directory provenance, or its wrapped prefix was evicted or
clipped by resizing, relative activation is refused rather than guessing a directory or file.

**Change Surround…** and **Delete Surround…** in **Edit > Text** (also in the command palette)
change or remove an existing `()`, `[]`, `{}`, single-quoted or double-quoted pair. Choose the
existing pair, a replacement for Change, and a positive outer pair count (default 1). Count 2
chooses the second enclosing pair of that kind: deleting it from `((value))` leaves `(value)`.
The entire selection must fit inside the chosen pair, including its delimiters; carets on a
delimiter or immediately after its closer work too. At adjacent pairs, that boundary prefers
the pair starting there. A reverse selection keeps its direction.

Matching balances mixed brackets, ignores brackets inside complete quoted strings, and treats a delimiter
after an odd number of backslashes as escaped. Quotes can span lines; word-glued quotes in
contractions are literal, and unmatched quotes leave subsequent bracket pairs reachable. Malformed or unmatched
pairs are left alone. These commands preserve the interior exactly, including existing escapes;
changing the quote style does not rewrite or escape its contents. Multiple disjoint pairs edit
in one Undo step; repeated targets edit once, while distinct nested targets reject the whole
command. Pure virtual targets do nothing. A form whose source or selections changed must be
reopened. Automatic wrapping and empty-pair Backspace keep their usual behavior.

### Selection sets

**Selection > Selection Sets** and the command palette offer four operations that change
selections while leaving document text untouched:

- **Invert Selections** selects the document outside the union of the selected physical ranges.
  Selecting `b` in `abcd` produces `a` and `cd`; carets and virtual space occupy no source.
- **Split Selections by Regex…** removes delimiter matches from each selected range. Splitting
  `a,b,c` on `,` selects `a`, `b`, and `c`. Empty pieces are omitted.
- **Keep Selections Matching Regex…** retains ranges containing a match.
- **Remove Matching Selections…** removes ranges containing a match.

Regex operations inspect each complete selected string independently. For example, `^cat$`
keeps a selection containing exactly `cat`; lookaround cannot inspect outside that selection.
They use Find's regex dialect and limits, default to case-sensitive, and offer **Ignore case**.
An empty regex is a zero-width pattern: splitting divides at Unicode scalar boundaries, including
between a letter and combining mark. Zero-width delimiters consume no characters. Invalid or
runaway patterns leave selections unchanged and report an error. A split without matches retains
the original selection; predicates preserve retained selections' direction, virtual endpoints,
rectangular origin and vertical-motion goal. Split pieces retain direction and rectangular origin,
with virtual space retained only at an original outer endpoint. Overlaps normalize as usual.

If no range remains, one physical caret stays at the previous primary head. The primary follows
its retained selection or last split piece; removing it chooses the preceding survivor, otherwise
the first. Inversion creates forward ranges and makes the first primary. **Undo Selection** and
**Redo Selection** traverse these changes; normal text Undo/Redo and the saved state remain intact.
Viewer, Follow and filesystem read-only documents permit these selection operations. Finish or
cancel IME composition first. Regex forms refuse submission if the source or selection changed,
including a selection moved away and back, or a shared view is composing. There are no default
shortcuts for these commands.

A split refuses the whole operation if any result would place an endpoint between CR and LF
in a CRLF line ending, including empty or zero-width patterns. Match the full `\r\n` delimiter
instead of only `\r` or `\n`. Other selected targets and selection history remain unchanged
when this error occurs.

### Adjust selected values

Select each complete value, then choose **Edit > Text > Increment Values**, **Decrement Values**
or **Adjust Values…** (also in the palette). Adjust Values defaults to `1` and accepts a signed
whole decimal step from −170141183460469231731687303715884105728 through
170141183460469231731687303715884105727. A caret alone does nothing. All selected values must
be supported and in range; one invalid value or overflow leaves every selection unchanged.
The successful batch undoes together. The Column Editor's generated numbering is separate.

Integers support an optional leading `+`/`-`, decimal digits or `0x`/`0X`, `0b`/`0B`, `0o`/`0O`
prefixes, and underscores between digits. Examples: `009` +1 → `010`, `0x0f` +1 → `0x10`,
`-001` +1 → `000`, `9_999` +1 → `10_000`. Arithmetic is checked signed 128-bit. Prefixes and
minimum digit width remain; underscores keep their positions counted from the right, so growth
expands the first group. An explicit `+` remains on nonnegative results; a minus disappears at
zero. Hex letters retain the case of an aligned source letter; new letters use the first source
letter's case (prefix case if there were none). Thus `0xaB` +1 → `0xaC`. A zero step validates
the selections and preserves their exact text, including negative zero. Spaces, floats and
integer suffixes are unsupported.

Dates use strict `YYYY-MM-DD`, Gregorian years `0001`–`9999`, and steps in days. Times use
`HH:MM` or `HH:MM:SS`, steps in minutes, and wrap within 24 hours while retaining seconds.
Date-times use `YYYY-MM-DDTHH:MM[:SS]`, optionally ending in `Z` or `±HH:MM` (offset hours
00–23/minutes 00–59). Minute steps adjust the local calendar fields and preserve the exact zone
and seconds. Dates/date-times refuse results outside the supported years. Lowercase `t`/`z`,
spaces, fractional/leap seconds, compact zones and invalid calendar/time fields are unsupported.

Moving the selection or changing the source after opening Adjust Values requires reopening the
form. Finish composition in another shared view first. Viewer and Follow refuse text edits;
normal filesystem read-only buffers keep the existing in-memory editing behavior. Empty and
pure virtual selections insert no padding.


### Align fields

**Edit > Columns > Align on Pattern…** aligns the starts of matching fields across source lines.
The default regex is `=` and capture index `0` means the whole match; choose a numbered capture
from your regex to align that group instead. For example, `a=1` and `long=2` become `a   =1`
and `long=2`. Every distinct matching field column is aligned in order. Lines without a match
stay unchanged; an optional unmatched or empty capture leaves its column slot empty. Nonempty
physical selections scope their covered lines; a selection ending at the next line's start
excludes that line. With ordinary carets, the command uses the document. Pure virtual selections
do nothing. Regex syntax and limits are the same as Find; Ignore case defaults off.

**Align Selected Fields** treats each nonempty same-line selection as a field. Select one or
several fields per row; corresponding field columns align their right edges by appending spaces
inside the selections. Reverse and rectangular selections retain their direction and origin.
Tabs expand at the source tab stops, wide/emoji clusters take two columns, and a combining
sequence takes one. Each later column is measured after earlier padding changes tab positions.

Both commands insert spaces, preserve existing content, delimiters, tabs and line endings, and
undo as one batch. Repeating a settled alignment changes nothing. Invalid patterns, capture
indices, overlapping or multiline selected fields, and boundaries inside a grapheme or CRLF
refuse the complete edit. A pattern form must be reopened after source/selection changes,
including moving away and back. Finish local/shared composition before aligning. Viewer and
Follow refuse edits; filesystem read-only buffers keep ordinary in-memory editing behavior.
Column Editor numbering, caret-only Pad to Column and CSV commands remain separate.

Pattern alignment treats leading spaces/tabs captured before content as existing padding. A
whitespace-only capture uses the end of its complete adjacent horizontal whitespace run, so even
`a( )` with capture `1` settles after one run. Multiple matches in the same padding run form one
normalized column, including whitespace matches whose chosen capture is absent; distinct
nonpadding missing/empty captures keep their slots. Coincident physical anchors form one field
boundary. Original spaces and tabs are retained.

A padding match with a missing capture and a following selected capture at that exact run end
share one boundary column; the selected capture fills that column. This keeps newly inserted
spaces from adding an extra field on the next run.

If inserted padding would change the pattern's field columns and require more padding on a
second run, the complete edit is refused with an explanation. Make the surrounding pattern
tolerate spaces: use `a\s*(=)` with capture `1` for `a= a=` / `xa= a=`. Literal `a(=)` would
lose a match after inserting between `a` and `=`, so that unstable alignment is refused. Valid
whitespace-consuming patterns remain supported; no partial edit is applied.

Pattern alignment includes the active snippet's mirror updates when checking whether the
result settles. If those updates would move fields out of alignment, the complete command
refuses without changing the text or snippet; finish the snippet before retrying. An ordinary
caret still gives document scope when other selections are entirely in virtual space.

### Find in comments or strings

The current-buffer Find/Replace panel has an **Any syntax** button. Click it to cycle through
**Comments**, **Strings**, **Comments or Strings**, and **Any syntax**. The adjacent
**Include/Exclude** button chooses those classes or their complement. Any syntax searches all
classes regardless of Include/Exclude. These panel choices reset to Any when the panel is
reopened; Find in Files keeps its separate options.

The class comes from the selected syntax's current parser or grammar scopes. A match belongs
to the class at its **starting character only**: a multiline regex or a match crossing from a
comment/string into code remains eligible if its start qualifies. Parser winning captures
apply inside interpolation: JavaScript template text is a string, while an interpolated
identifier is code. Grammar fallback uses its full scope stack, where an enclosing comment
wins over string and a retained enclosing string still counts as string. Escapes and embedded
languages follow those same grammar scopes. Newline positions inherit the preceding character's
class; an empty line uses the grammar's base class.

Syntax filtering composes with case, whole word, regex, selection scope and Preserve Case.
Find Previous/Next/All, Replace/Replace All, extraction, bookmarks and highlights use the same
eligible matches. Replacement uses the normal single undo step. Viewer/Follow sources and any
shared view's active composition refuse syntax-filtered replacement before changing source or
history; filesystem read-only files retain their ordinary editable working-copy behavior.

While classification is preparing, or if the selected language cannot classify the document,
the panel says matches are unavailable and replacements do nothing. It never falls back to an
unfiltered search. Edits cancel the old classification and refresh after a short typing pause;
a syntax change also invalidates it. Plain Text, sources over 16 MiB, lines over 20,000 UTF-8
bytes, more than one million class runs, parsing errors or a two-second classification limit
are unavailable. Choose Any syntax to use ordinary Find for such documents.

An invalid replacement template prevents replacement. Find All, extraction, bookmarking and
persistent highlights can still use a valid query and its ready syntax-filtered matches. The
replacement error stays visible while the panel is open; pending or unavailable syntax
classification continues to refuse those search actions with its explanation.

### Passive selection occurrences

Enable `"selection_occurrences": true` in **Preferences > Settings** to shade other occurrences
of one nonempty single-line selection, up to 1,024 Unicode scalars (at most 4 KiB UTF-8).
Larger selections clear the overlay and start no worker. It is off by default and can also be
set in project, language or path preferences. Matches are literal, case-sensitive Unicode text;
punctuation has no regex meaning. Like Find, matches do not overlap: `aa` in `aaaaa` matches characters
0–2 and 2–4. The selected match itself is omitted.

Each view keeps its own overlay, including read-only and viewer views. Clearing the selection,
selecting several ranges or logical lines, virtual columns, composition, or disabling the
setting clears the overlay. Changing text/selection cancels old work; only current results
appear. Searches use Find's background deadline. Folded matches stay hidden and visible matches
follow soft wrapping. Text, selections, Undo, Find and all five manual highlight styles are
unchanged.

### Search within previous matching lines

In Find in Files, open **Where…** and choose **Previous Matching Lines**, or enter
`<previous matching lines>` in Where. First run a source search and wait for it to finish
(or cancel it to keep the results found so far). The next query searches only the whole
physical lines touched by those source matches. Context rows, inclusion checkboxes and the
results browser's path/text filter do not change this scope. `<previous results>` still
searches the whole files that matched.

For example, search for `TODO`, then choose Previous Matching Lines and search for `owner`:
only `owner` on the original TODO hit lines qualifies. Adjacent hit lines form one interval,
so multiline matches may cross their line break. A match cannot extend into an intervening
non-hit line or join disconnected intervals. Regex anchors, lookaround and whole-word checks
still see the complete source. Following the existing selection-scope rule, an empty regex
match at an allowed interval's end is included.

Changing the query, matching options or browser filter, and using **Refresh** on the new
results, keeps the original chosen hit lines. Choosing Previous Matching Lines again explicitly
captures the currently displayed completed result set. An empty source result set reports that
there are no hit lines. Use this scope by itself, optionally with include/exclude globs;
combining it with other source paths or scope tokens is refused.

Open unsaved buffers retain their exact document/version and text snapshot, including shared
views; disk sources retain their actual encoding, BOM, line endings and disk identity. The
chosen source stays the same when Use Buffer is toggled. If it is edited, closed, renamed,
reinterpreted, deleted, rebound through an alias or overlapped by a file write, that file reports
stale lines and produces no new matches. Other unchanged files can still be searched. Refresh
the original source query, then choose its Previous Matching Lines again to renew stale bounds.

Replace… previews only complete eligible matches. Apply rechecks source freshness, read-only
and shared native composition protections before changing an open buffer or writing a file.
Open-buffer changes stay unsaved with one undo step; closed-file writes preserve their decoded
codec, BOM and line endings through the ordinary checked writer. An invalid replacement
template prevents replacement while a valid **Find** query remains usable.

The window keeps this chosen snapshot even if the original query is refreshed or another search
publishes results. To change the chosen set, use **Where… > Previous Matching Lines** again.


### Search result pages

Each Find in Files or Replace in Files search creates its own structured page in this window.
Open **Find > Search Results** to revisit them. The buttons above the browser select a query;
its adjacent **×** closes that page. Each page keeps its query, replacement, options, resolved
scope, source versions, checkboxes, collapsed files, filter and review note. Filtering changes
what you see; checkboxes choose what Apply changes. Apply always uses the selected page and
refuses stale sources through the existing preview checks. While Apply is running, wait for
it or Cancel before switching, closing, refreshing or starting a search.

**Refresh** reruns only the selected page with its original options and chosen scope, replacing
that page's preview and checkboxes. A Previous Matching Lines page retains its exact frozen
source lines, even after you choose a different basis elsewhere. To select new source lines,
rerun the source query and explicitly choose Previous Matching Lines again.

A window retains up to eight pages. A ninth search retires the oldest inactive page whose
search has ended; if no page is eligible, the new search refuses with an explanation.
Starting a search cancels the prior running search and retains its partial results. Switching
pages lets that single search finish on its own page; closing its page cancels it. Closing the
last page hides the browser. Ordinary Find in Files still appends to the textual **Find Results**
tab, with its existing result navigation; closing structured pages never clears that text.
Pages are transient and independent between windows.

### Narrowing a view

Use **Edit → Narrow View → Narrow to Selection** to show only your selected source ranges,
or **Narrow to Fold** to focus the containing active fold or current syntax/indent block,
including its header. Ordinary folds open temporarily so the focused text is readable.
**Show Matching Lines…** takes a regex and a nonnegative whole **Context lines** count
(default **0**). It is case-sensitive unless **Ignore case** is checked. Multiline matches
show every line they touch; context is clipped at the document edges and overlapping runs
are combined. The command palette exposes the same commands under **View**; all four typed
`editor::NarrowToSelection`, `editor::NarrowToFold`, `editor::ShowMatchingLines` and
`editor::ClearFilter` commands are available to custom keybindings and macros. A direct
matching-lines command takes `pattern`, optional `context` (0), and `ignore_case` (false).

Hidden text remains in the document behind fold placeholders. Editing, selection, copy,
line numbers and Find continue to use the full source's positions. Focus ranges follow edits,
including text inserted at their edges. Matching lines update after source changes, including
changes from another view; the full current source is shown while a new scan runs. The lines
holding the carets stay shown after an edit even when they no longer match, so text you are
typing never disappears or moves the caret. A first scan with no matches shows a whole-document
placeholder and a caret at the start, without deleting text. Invalid
regex/context leaves the previous view intact; a failed scan restores the prior folds and
reports the error.

**Clear Filter** restores your previous nested and inline folds, mapped through real edits.
Changing filter modes keeps the original fold snapshot. Ordinary fold commands clear the
filter first; navigating to a hidden source selection also clears it and reveals that
selection. Filtering itself does not change text, dirty state or text Undo/Redo. Other views
keep their own presentation. Read-only, Viewer and Follow views allow narrowing. Finish or
cancel text composition before changing filters; an active composition temporarily suspends
exclusions, and a delayed regex result waits for a later frame/action after composition ends.
Sessions and reopened closed views retain the prior ordinary folds only. Filters, regexes,
context counts and generated exclusion folds are transient.

Moving selections while a matching-lines scan is pending cancels that result, including moving
away and back. Run the command again to filter the new selection context. Navigation to hidden
text restores prior folds with the actual source destination revealed.


## Fold navigation

**Edit > Fold Navigation** and the command palette provide **Move to Next Fold**,
**Move to Previous Fold**, **Move to Parent Fold**, their **Select to…** variants, and
**Select Fold**. Custom bindings and macros use `editor::MoveToFold` with `motion` set to
`next`, `previous` or `parent` and optional `select: true`; `editor::SelectFold` takes no
arguments. These commands have no default keys.

Navigation uses current syntax fold candidates, or indentation blocks when syntax folding is
unavailable, together with your view's collapsed manual and inline selection folds. Expanded
syntax/indentation blocks also count. Multiline headers start at the physical line start;
inline folds start at their exact selected character. Next/previous go strictly forward/backward
from each caret to a different header, without wrapping. Previous from an interior reaches
that fold's header. At either document boundary, a missing destination leaves that selection
unchanged; Previous from EOF can reach the last header.

The current fold is the smallest source range containing the caret, including its header.
Its end is outside unless it is document EOF. Parent means the nearest range strictly enclosing
that current fold; an outermost fold has no parent. Ranges sharing a header can have that same
header as their parent destination. Select Fold includes the current multiline header and body,
or the exact inline range, keeping selection direction. Select to variants keep the existing
anchor and extend to the destination header. Multiple carets use the same rules independently.

Navigation changes selections and reveals hidden destinations with normal scrolling; it never
changes source text or inserts virtual padding. Unrelated collapsed folds stay collapsed.
Undo Selection/Redo Selection track these commands; text Undo/Redo stay available. In a narrowed
view, navigation uses the saved ordinary folds, never filter placeholders. A hidden destination
clears the filter and reveals that source location; a visible destination keeps the filter.
Read-only, Viewer and Follow views allow navigation. Finish or cancel text composition in any
shared view first. Fold navigation is available in document views, outside fields and aligned
comparisons.

### Find bookmarks across open files

Choose **Goto > Bookmarks > Find Bookmark Across Open Files…**, or the matching command in
the command palette. The picker lists bookmarks in every open window, with the filename,
source line number and up to 160 characters of text. Type a filename or text from the preview
to filter; press Enter to reveal the bookmark in its existing view and window. Shared views
of the same document and line appear once. Untitled documents are included.

Rows refresh while the picker is open. If the source or query changes immediately before
Enter, choose a current row again. Closing one bookmarked view uses another bookmarked view
when available; closing all owners removes the row and never reopens a file from disk.
Finish or cancel composition in the target document's views before jumping. Existing F2 and
Shift-F2 still move between bookmarks in the current view. For a custom binding or command
file, use `workspace::GotoBookmark`; it has no default shortcut or new preference.


Use **Goto > Open Selected > Open Selected Path/Address** to open exactly one selected
filename, optionally followed by `:line` or `:line:column` (positive, one-based; positions
clamp to the file). Single or double quotes may surround the path or full address. Spaces,
Unicode, extensionless names and punctuation are preserved: `"notes/雪 draft!":12:3`.
Relative paths use the current named file's parent; an untitled file needs exactly one
project folder. Absolute paths and local `file://` URLs also work. Missing files, folders,
ambiguous/empty/virtual selections, invalid locations and unmatched quotes report a status
message without creating a file. Multiple project folders require an absolute path for an
untitled source. A later source edit, selection/tab/project/focus change or composition
cancels a pending navigation. Existing open documents and remembered encodings are reused.

**Open Selected URL** explicitly opens a selected `http://` or `https://` URL in the default
browser. The path action never launches a web URL. Both actions are available in the command
palette and custom bindings as `workspace::OpenSelectedPath` and `workspace::OpenSelectedUrl`;
neither has a default shortcut. Ordinary selection, Copy, result links and mouse clicks keep
their existing behavior.


### Temporarily protect a document

Choose **File → Toggle Viewer Mode** or **File: Toggle Viewer Mode** in the Command Palette. Viewer mode protects every shared view of the document from typing, paste, Undo/Redo, replacement, transforms, pipelines and server edits. `[VIEWER]` appears in the window title and tabs, including when the status bar is hidden. **VIEWER · Unlock** in the status bar stays visible during status messages; click it to turn protection off. Selection, navigation, Find and copying still work. Finish or cancel composition in every shared view before toggling.

Existing unsaved text and Undo history are retained. Save and close keep their normal behavior; saving protected dirty text skips formatting and cleanup. Turning viewer mode off removes only this temporary protection, so snapshot viewers, Follow and disk overwrite restrictions still apply. The flag resets when the document closes and is not restored with a session. There is no default shortcut; bind `workspace::ToggleViewerMode` in custom keys or commands.


### Mixed line endings

Orlo recognizes LF, CRLF and CR file boundaries before displaying normalized source. Opening
and saving keeps each existing boundary, including a missing final newline, the encoding and
BOM. Editing other text leaves those boundaries intact; newly inserted newlines use the
document's preferred ending (detected majority, then existing repository/explicit preference).
Automatic save preferences do not convert existing boundaries.

Use **Edit > Inspect Line Endings…** or **Document: Inspect Line Endings…** in the palette for
LF/CRLF/CR counts and the final-newline state. Choose a kind to navigate to its next boundary,
wrapping at the end. The corresponding **Document: Next … Boundary** commands can be bound to
keys. Locations refer to normalized source and work in Viewer and Follow modes.

**View > Convert Line Endings** and the three **Document: Convert Line Endings to …** palette
commands explicitly convert every boundary. Conversion is one shared Undo step, including when
only the physical endings change. It also normalizes literal CR/CRLF introduced by a text
operation, so converting to CRLF cannot retain an extra CR prefix. Choosing an already uniform
current ending records an explicit preference as an Undo step while keeping the file clean;
choosing it again is a no-op. Finish native composition in all shared views first;
Viewer/Follow and immutable comparison views refuse conversion.

Save a Copy, queued saves, Undo/Redo, recovery and encoding conversion retain the captured
boundary map. Older recovery records without a map initially use their saved single-ending
default, then retain those boundaries through later edits and preference changes. Invalid
new maps are isolated and privately retained with the damaged session, never silently used to
overwrite a source. Recovery maps are bounded to 100 MiB of compact metadata.

Clipboard and arbitrary text transformations keep their established CR normalization rules;
literal CR left in source remains literal on ordinary save. File History restores text only:
unchanged destination boundaries retain their kinds and inserted ones use its preference.
Revert and Follow ingest the actual file's boundaries. Historical backups retain raw bytes.


Discarded Untitled rescue is text-only **Restore as New Untitled**: it creates an independent
new document with default output format, keeping the normalized source text. It does not
restore encoding, BOM, preferred ending or the original boundary map. Hot-exit session recovery
retains that document metadata; File History keeps the destination's format as described above.

## Insert Unicode and literal keyboard text

Choose **Edit → Insert Unicode Character…** (also in the Command Palette) and enter one
hexadecimal Unicode scalar, such as `U+1F600` for 😀 or `0301` for a combining acute accent.
The `U+` prefix is optional. Surrogates, malformed values and values above `10FFFF` show an
error without changing the document. The character replaces every selection in one Undo step.
If the source or selections change while the form is open, reopen the command. The JSON
action is `editor::InsertUnicodeCharacter` with `{"value":"U+1F600"}`.

**Edit → Insert Literal Next Key** arms the focused document for one text-producing key,
bypassing its editor binding and automatic bracket/quote pairing. Escape cancels. Navigation
and Control/Command/Option/function keys cancel and retain their ordinary meaning; shortcuts
never become raw control characters. Native IME/dead-key composition takes priority and
cancels arming while preserving its marked range and normal commit. Literal arming yields
entirely to active input methods and native layout dead keys; selecting another input source
cancels it. Blur or keyboard-mode
changes cancel arming. A changed document cancels and consumes the pending input, preserving
late composition in another shared view. Read-only, Viewer, Follow and immutable views refuse
these commands; finish composition in all shared views before starting them. Return/keypad Enter insert a plain newline without autoindent; Tab inserts a literal tab.
Macros store the resolved literal insertion, so cancellation leaves no armed mode in the
recording and replay does not depend on the current input source. The arming action is
`editor::InsertLiteralNextKey`, available for custom bindings.

**Edit → Character Viewer** and **Edit: Character Viewer** open the existing macOS picker.
The standard **Emoji & Symbols** menu and **Control-Command-Space** keep using the same OS
input service. Native character selection, IME input and Undo keep their ordinary behavior.
