# Orlo

A code editor with Sublime Text's editing model, written in Rust on [gpui](https://gpui.rs), Zed's UI
framework. Sublime's layouts with a tab strip per group, folder sidebar, minimap, status bar,
Goto Anything, Goto Symbol, Goto Symbol in Project, Command Palette, find/replace, Find in Files, word wrap, bookmarks,
auto-paired brackets and Sublime's multi-cursor editing, with the Mariana colour scheme. Editor
geometry and command behaviour were sampled from Sublime Text 4 on this machine. macOS first.

The chrome is Orlo's own: a light and a dark appearance (picked like Sublime's `"theme": "auto"`,
by the macOS appearance) built from one cool neutral ramp and a blue accent, each text and icon
pair measured for contrast. The tab strip is drawn from the colour scheme, a step darker, so it
reads as the editor's edge under any scheme, with an accent line on the tab that takes input.
Icons with tooltips instead of text buttons, rounded selection pills in the side bar and pickers,
a one-row find bar with its options inside the field, and git status as tinted names. The macOS
menu bar follows Sublime's menus, with Terminal and Git promoted to menus of their own.

Orlo can be customized and automated through editable keybindings, scoped settings,
EditorConfig and indentation detection, installed themes/grammars, macros, local shell packages,
filters, build tasks with navigable output, shell worksheets and a developer console.
See [Customization and automation](docs/user-guide/customization.md) for formats, commands and limits.

## Product scope

Orlo targets Sublime Text's editing behavior and interface, with an integrated terminal,
Git and a diff viewer, plus selected features from similar text editors. Beyond those it has four
[text essentials](docs/user-guide/text-essentials.md): CLI waits and stdin, Unicode encodings, optional
historical backups, and JSON tools. Other capabilities of those editors are left out, so Orlo
stays a text editor. Tree-sitter and local language servers cover seventeen languages, and a
language folder adds another without rebuilding Orlo; see [Language intelligence](#language-intelligence).

## Project status

Orlo 0.1.0 is ready for local macOS alpha testing. Start with the
[local alpha guide](docs/user-guide/local-alpha.md) for launch instructions, a testing checklist
and recovery steps. [Not in this version](#not-in-this-version) lists the feature limits. The
[alpha readiness assessment](docs/development/release-readiness.md) records verification and release
status, and the [source map](docs/architecture/overview.md) explains where to make changes.

## Run

The Cargo workspace contains ten crates. `orlo-app` is the default member, so `cargo run`
also selects the app. See the [architecture map](docs/architecture/overview.md) and
[development checks](docs/development/verification.md).

```sh
cargo run -p orlo-app --release -- .                       # open this folder in the sidebar
cargo run -p orlo-app --release -- crates/app/src/main.rs  # open files
./scripts/bundle.sh                                      # build target/Orlo.app
open target/Orlo.app                                     # launch the local alpha
```

`scripts/bundle.sh` signs the app ad-hoc, which runs on the Mac that built it. For a download that
passes Gatekeeper, sign with a Developer ID and notarize; see
[distribution](docs/development/release-readiness.md#distribution).

One Orlo runs at a time, as with `subl`: launching it again (or `open -a Orlo file`, or a drop on
the Dock icon) hands the paths to the running one and exits. A lock on
`~/Library/Application Support/Orlo/Orlo.lock` decides which one runs, so two launches at once
cannot both restore the session.

Syntax highlighting uses Sublime's own grammars through `syntect`. gpui is built with its
`runtime_shaders` feature, so a full Xcode install is not required.

## Menu bar

Orlo, File, Edit, Selection, Find, View, Goto, Tools, Project, Window and Help mirror Sublime's
macOS menus, item for item where Orlo has the feature; Orlo adds View > Groups, Tabs, Panel, Sync
Scrolling, Word Wrap Column and Enter Distraction Free Mode, and Project > Side Bar Item. File > Open Recent lists the files and
folders opened in any window, kept across launches; View toggles the minimap, tabs, status bar, white space marks and side bar, holds Layout, Focus
Group and Move File to Group, switches syntax, line endings and the ruler, and has Word Wrap and spelling commands; Edit holds Copy as HTML, Paste from History, Clear Clipboard History, Code Folding, Convert Case, Convert, Transform, Wrap, Show Completions, Sort,
Permute, Columns, Document Statistics and Character Info; Selection holds Undo Selection and Redo Selection; Find holds Quick Skip Next beside Quick Add Next
and ends with Find in Files and Cancel Find in Files; Goto holds bookmarks and scrolling; Tools holds the Command Palette, Snippets, Text Pipelines and Compare. Every menu item is also in the
Command Palette.

## Layouts and groups

View > Layout splits the window into Sublime's layouts: Single, Columns 2–4, Rows 2–3 and Grid 4.
Each group has its own tab strip; the active group's view is the one commands act on, and the
others are dimmed. Click into a group to focus it, or drag the side bar edge or a group divider to
resize. Goto Anything opens a file in the focused group, as a second view when it is already open
in another group; "File: New View into File" in the Command Palette adds one next to the current
tab. All views of a file share its text, undo history and save state, in every window, and keep
their own carets, scroll position and bookmarks. Closing a group's last tab leaves the group empty.
The Find and Replace panel runs along the bottom of the whole window and acts on the focused
group.

View > Groups splits the focused group right or down (Split Right, Split Down), moves the current
file into a new group split off its own (Move File to New Group Right / Down), and closes a group
(Close Group): its tabs, unsaved ones included, go to the neighbour that grows over its place, its
active tab staying active there. Splits nest, so one tall group can sit beside two stacked ones. A
split that would leave a group under 120 by 90 points is refused, and so is closing a group no
neighbour fits exactly (a pinwheel of five groups); the layout stays as it was. Dividers are drawn
and dragged only where two groups meet; a drag moves that grid line for every group on it and
stops 24 points from the next one. View > Layout > Save Layout… keeps the current geometry under a
name in `~/Library/Application Support/Orlo/Layouts.json` (a hand-edited file is checked, and an
entry whose cells do not tile the window is left out and named); saved layouts are listed in
View > Layout, and Apply / Delete Saved Layout… pick one from a list. Applying one with fewer
groups moves the extra groups' tabs into the last one, as the presets do.

View > Sync Scrolling > Vertical and Horizontal make the views on screen follow the one you scroll
(wheel, trackpad, Page Up/Down, the minimap, scroll commands): vertically to the same line (a
shorter file stops at its end; wrapped or not, a line is a line), sideways to the same offset. A
view following never moves the others in turn. The choices are kept with the window.

## Tabs

`⌘`-click tabs to show them side by side in their group (`⇧`-click shows a range); the one with the
input has an orange line along its top and the others are dimmed. A click into one of them gives it
the input; a plain click on another tab, or Tab > Deselect Other Tabs, goes back to one. The
keyboard has Tab > Add Next / Previous Tab to Selection (`⇧⌥⌘→` / `⇧⌥⌘←`) and Focus Next / Previous
Selected Tab (`⌘K ⌘]` / `⌘K ⌘[`). Nothing of this changes the layout or is saved; closing a shown
tab gives the input to another.

Drag a tab to reorder it, onto another group's strip or area to move it there (an empty group
too), onto another Orlo window, or out of every window into a new one; a line marks where it will
go and the strip scrolls near its ends. Escape, or letting go anywhere else, leaves it where it
was. A moved tab is the same view: its unsaved text, undo history, selections and scroll go with it
and another view of the file elsewhere keeps following it. Hold `⌥` while dropping to add a new
view of the same file instead (it shares the text, never a copy). Dragging one of the tabs shown
side by side moves them all. Tab > Move to New Window does the same from the keyboard. A window
whose last tab leaves and that has no folders closes; Orlo carries on in the other one.

A single click on a file in the side bar opens it as a **preview**, its name in italics: the next
file clicked replaces it. It is kept once edited, double-clicked (in the side bar or on its tab),
dragged, opened another way, or with Tab > Keep Open (`⌘K ↩`). A preview with unsaved changes (or
whose file was deleted) is never replaced. Goto Anything keeps its own live preview.
Tab > Pin Tab keeps a tab at the start of its strip with a pin; Close Other Tabs, Close Tabs to
the Right and File > Close All Files leave pinned tabs open (close one with `⌘W` or its `×`,
which asks about unsaved changes as usual). Preview and pinned tabs are kept in the session.

The `▾` at the right of each strip lists that group's tabs; Tab > Tab List… (`⌘K ⌘T`) lists the
window's by group, and Tab > All Open Tabs… every window's. Each row says which tabs are active,
shown, unsaved, pinned or a preview; files of the same name show their folder, and more views of
one file are numbered. Type to filter, Enter goes to the tab, `⌘⌫` closes it (asking first when it
is unsaved). View > Side Bar > Show Open Files adds Sublime's OPEN FILES section above the folders,
by group, with the same rows, kept up to date as tabs move, close, get renamed or are kept. Right
click a tab for Close, Close Other Tabs, Close Tabs to the Right, Pin, Keep Open, New View into File
and Move to New Window.

## Folders and projects

Project > Add Folder to Project… (or opening a folder) adds a folder after the ones there; a
folder already in the window is shown, not added twice. Each folder is a top-level row of the side
bar, expanded on its own; Remove Folder from Project… (the selected row's folder, or a list), Move
Folder Up / Down and Remove all Folders from Project change them, and the files open from a folder
removed stay open, edits and all. Goto Anything lists every folder's files (the file name, then the
folder's name and the path in it), a file reached through two folders (one inside the other, or a
link to the same place) once. `<open folders>` in Find in Files' Where, and globs alone, search
every folder. The status bar shows the branch of the active file's folder. A folder that is
deleted, renamed or on a drive that is not mounted stays in the side bar as "name (not found)",
dimmed, and comes back when it is there again.

A **project** is a `.sublime-project` file, as Sublime writes them:

```jsonc
{
	"folders": [
		{ "path": ".", "name": "App", "folder_exclude_patterns": ["target"], "file_exclude_patterns": ["*.log"] },
		{ "path": "../shared" },
	],
	"settings": { "tab_size": 2, "word_wrap": true, "wrap_width": 100, "rulers": [100] },
}
```

Folder paths are relative to the project file (or absolute, or `~/…`). Exclude patterns are
`.gitignore` lines read from the folder (a name hides it at any depth; a path with `/` from the
folder), hiding entries from the side bar, Goto Anything and Find in Files. `settings` takes
editor font, indentation, wrapping, rulers, minimap, animation and save-cleanup preferences,
plus scoped `overrides`; app-wide appearance and unsupported keys produce a warning. When Orlo writes the file (adding or removing a folder, Save Project
As) it keeps the keys it does not use (`build_systems`, say), the way each folder's path was
written while it still names that folder, and new folders inside the project's folder relative;
comments in the file are not kept. A file that does not parse is shown with its error and the
window keeps what it had.

Settings apply in this order: defaults and indentation detection, global preferences and
matching language/path/scope rules, project preferences and its scoped rules, optional declarative
modelines, EditorConfig, window menu choices, explicit buffer indentation, and per-view wrapping.
Explicit buffer/view choices survive preference reloads. See [Customization and automation](docs/user-guide/customization.md).

Each project has a **workspace**, `<name>.orlo-workspace` beside it: the window as you left it
(layout, groups, tabs, carets, scroll, bookmarks, pinned and preview tabs, panel places, the side
bar) and the text of its unsaved and untitled files. It is private (mode 0600) and is best kept
out of version control. Sublime's own `.sublime-workspace` files are never written.

Project > Open Project… opens a project or workspace file; Switch Project… (`⌃⌘P`) lists the recent
ones (Project > Open Recent too). A project open in another window brings that window to the
front instead. Switching writes this window's workspace first, unsaved and untitled text
included, then shows the other project; nothing is asked, and switching back brings it all back
unsaved. If the workspace cannot be written, the window keeps its project and says why. A window
with no project opens one in a new window unless it is empty. Save Project As… writes the
window's folders to a new project (and its workspace), Save Workspace As… a second workspace of
the project, Close Project writes the workspace and empties the window, and Edit Project opens the
project file: saving it applies it to its window. Project and workspace files given to Orlo from
Finder, the Dock or `Orlo <file>` open as projects. A workspace file that does not read is moved
aside as `<name>.orlo-workspace.bad` and the project opens without it.

## Side bar

Click a folder to expand it, a file to preview it (double click to keep it); the keys stay in the
side bar. `⌃0` (View > Side Bar > Focus Side Bar) gives it the keys too: Up and Down move, Right and
Left expand and collapse (Left on a file goes to its folder), Enter opens a file to keep (Space
previews it), F2 renames, `⌘⌫` moves to the Trash, Escape goes back to the editor. `⌘`-click marks
several rows.

Global Settings accepts `"explorer_sort": "natural"` to order filenames such as `chapter-2`
before `chapter-10`; `"lexical"` keeps the default case-insensitive lexical order. Folders
always precede files. Natural digit runs have no integer-size limit; equal values prefer fewer
leading zeros, with case ties resolved by the original filename. The separate
`"explorer_sticky_ancestors": true` setting keeps parent folder rows visible while scrolling;
it defaults to false. Pinned rows support the same clicks, marks, context menus and keyboard
browsing as tree rows. Short viewports keep the nearest ancestors and leave a content row.
These preferences apply to the tree in every window; project and document overrides cannot
change them. Path Breadcrumbs child menus keep their ordinary lexical ordering.

The side bar's Back, Forward and Parent controls navigate folder locations without replacing the
project's roots or collapsing other expanded branches. With the side bar focused, use `⌥←`,
`⌥→` and `⌥↑`; the same Explorer commands are in View > Side Bar and the command palette.
Each window remembers at most 32 locations including the current folder. Visiting a new folder
after Back replaces the forward branch. Deleted or replaced folders are skipped, and moves made
in Orlo follow their new paths. Parent at an outer project root says so and keeps every root.
This transient folder history is separate from the editor's Jump Back / Forward history.
Custom keymaps can bind `workspace::ExplorerBack`, `workspace::ExplorerForward`,
`workspace::ExplorerParent` and `workspace::PathBreadcrumbs`.

**Path…** (View > Side Bar > Path Breadcrumbs…, or Explorer: Path Breadcrumbs… in the palette)
shows the active file's ancestors in a searchable menu. Pick an ancestor to browse its immediate
children, then open a file normally or reveal a folder in the existing tree. Revealing a folder
outside the project asks you to add it explicitly. Filesystem reads run in the background; a
changed source, project or replacement menu discards delayed work. A child deleted or replaced
while opening is refused; a file being written asks you to try again. These commands read paths
without modifying dirty text, viewer state or native composition. The menu hides the explorer's built-in metadata and temporary save names, with folders listed
before files in lexical order.

Right click a row (or use Project > Side Bar Item) for New File…, New Folder…, Rename…, Move…,
Duplicate…, Move to Trash, Reveal in Finder and Find in Folder…; a folder's own row also has Remove
Folder from Project and Move Folder Up / Down. Each asks in a panel at the bottom that says what it
will do (the folder something goes in, the file renamed), and an error (a name taken, no
permission, another volume) is shown there with nothing changed. Nothing is ever written over: a
name in use is refused, but changing only the case of letters works. Move… takes the marked rows
when the clicked row is one of them, and moves each to the folder typed (a path, `~/…`, or relative
to their folder); it never crosses volumes (copy with Duplicate, then trash the original).
Duplicate copies the file or folder **as saved on disk**: unsaved changes in Orlo stay in their
own tab (the panel says so when there are some). New View into File is the way to a second view of
the same text. A link is renamed, moved, copied and trashed as a link; what it points to is never
touched, and the files open through it stay as they are.

A file open in Orlo that is renamed or moved, or is inside a folder that is, keeps its tab, its
unsaved text and its undo history in every window, under the new name; so do its other views.
Folders of the project inside a moved folder move with it (and the project file follows), the
expanded folders, marks and selection follow, Open Recent, Reopen Closed File and the session use
the new paths, and Goto Anything walks again. If another app changed the file just before, the
change is still noticed at the next check. A Replace in Files preview over a moved file shows it
stale ("moved or renamed since the preview") and Apply leaves it alone until Refresh. Move to
Trash uses the macOS Trash (Finder's Put Back returns it); files open from it stay open, marked
deleted, their text kept. File operations wait while Replace in Files or a pipeline run is writing.

## Panels

The results browser and the regex playground are panels you can resize by dragging their edge and
put along the bottom or the right (View > Panel > Move Panel to Bottom / Right); each keeps its own
place and size in the window and its workspace. The Find, Replace, Find in Files and form panels
stay along the bottom at their own height. View > Panel > Show / Hide Panel (`⌘K ⌘J`) hides the
panel without closing it (with none open, it shows the last search's results), Collapse Panel
shrinks it to its title (click it to expand), and Focus Panel moves the keys into it or back to the
editor. A panel moved, hidden or collapsed is the same panel: its fields, results, selection and a
search or Apply under way carry on. When it hides with the keys in it, they go to the editor. A
panel is drawn at its chosen size as far as the window allows, leaving the editor at least 120
points high and 200 wide.

## Terminal and Git

The `>_` button at the left of the status bar, Terminal > Show / Hide Terminal, `` ⌃` `` or `` ⌘` ``
toggles the bottom terminal: a login shell in the active file's folder, or the first project
folder. It takes normal terminal keys, `Ctrl-C`, IME text and `⌘V` (pastes up to 1 MiB); drag
to select and `⌘C` to copy, with 10,000 lines of scrollback. Hiding it, opening another panel or
collapsing it keeps the same shell. Close Terminal (`` ⇧⌘` ``) ends it, and showing it again
starts a fresh one. Shells and transcripts are not restored after quitting. Applications that
request mouse input receive presses, releases, motion and wheel events in their requested
X10/VT200/motion and default/UTF-8/SGR modes. Hold Shift before dragging to select terminal
text instead. Search results also remain a text-selection surface. Mouse coordinates use the
painted cell grid; moving within one cell does not send duplicate motion events. Releasing
a reported drag outside the output uses the nearest terminal cell, and losing focus ends the
reported drag. Shift-wheel keeps local scrolling available. A terminal reset
or shell exit disables mouse reporting; exit keeps the final output and selection available
for review and copying.

Use the terminal header's search button or `⌘F` while the terminal is focused to find literal,
case-sensitive text in its retained output. Enter / Shift-Enter and `⌘G` / `⇧⌘G` move through
matches and wrap at the ends; Previous and Next buttons do the same. Search includes soft
wraps and Unicode text. Output keeps running while a worker searches a screen snapshot;
results refresh at most four times per second as output arrives. `⌘C` with the terminal focused
copies a selected match. Close search or press Escape in its field to retain the displayed output;
choose Live Output or type to return to the live shell.

Cmd-click terminal output to open an `http://` / `https://` URL or a file path, including
`path:line:column`, `path:line`, `path(line,column)`, compiler-style `path:line:column:` and
quoted paths or whole locations containing spaces (`"my file.rs":2:3`, `"my file.rs:2:3"`).
Explicit locations also identify extensionless files such as `Makefile:12:3`; bare words remain unlinked.
Locations are one-based source characters and clamp to the file's text; missing files show a
status message without creating a document. Relative paths retain the local OSC7 directory reported when that output was printed (the launch directory
until the first report), including scrollback, paused Find, retained-close output and command
review. Unknown or mixed directories disable that relative link; absolute paths and web links remain available. Directory
history is bounded to the configured retained row count plus screen rows and 4096 print spans,
so very old links may become unavailable. A clipped or evicted token is not treated as a complete
relative path; complete links survive benign resizing. Path storage retains 4096 directory
transitions, and command-review provenance has a separate 1 MiB budget. Heavy directory/cursor
churn can expire old relative links; exhausted IDs fail closed until a new terminal session.
Remote OSC7 hosts and malformed URLs are ignored. File links open
through the regular editor, including its shared-document and large-file handling.

Use **Terminal: New** (Terminal > New Terminal or the header's **+**) for another independent
shell. Click a terminal tab or use **Terminal: Next / Previous** to switch. Hiding the panel
retains every session; Close Terminal ends the selected session and selects its neighbour.
In the tab strip, **×** closes that session without switching away from another active tab.
Use the header's **Rename…**, Terminal > Rename Terminal or **Terminal: Rename…** in the palette
to give the selected session a window-local name. Names accept up to 128 Unicode characters;
surrounding whitespace is trimmed and control characters are refused. Submit an empty name
to restore its directory label. Renaming keeps the same shell, output, cwd and search state.
Names and live sessions are discarded when their window closes; they are not restored on launch.
Changing projects or closing the window ends all its sessions. A cancelled window close or a
close waiting for source saves keeps them alive. Use **Profiles…** in the header or **Terminal:
New with Profile…** to launch another session with a named shell/argument/directory profile.
**Terminal: Set Scrollback…** changes the selected session's live history limit; reductions discard
oldest history and obsolete Find/review selections while the shell keeps running. Launch
preferences and live trimming are described in the [customization guide](docs/user-guide/customization.md#terminal-launch-preferences).

**Terminal: Send Selection** pastes selected editor text into the chosen terminal session without
adding a newline; **Terminal: Send and Run** adds one Enter after the paste. Nonempty physical
selections join in document order with newlines; empty carets and virtual-only selections are
skipped. Both use ordinary terminal paste rules: bracketed paste frames the text when supported;
otherwise embedded newlines become Enter bytes and can execute commands. A selected trailing
newline stays part of the paste, and Send and Run still adds its explicit Enter.
The editor text, selections, saved state and Undo/Redo stay unchanged, including in read-only
shared views. Both commands appear in Tools, show a hidden chosen session and create one only
when no session exists. Empty selections do not create a terminal. A dead session reports an
error without replacement; refused input preserves retained output review.
Finish or cancel text composition in every shared view before sending. If a shared edit is
updating the selection, try again after the view updates.

Set `terminal.shell_integration` to `true` in global Preferences to enable temporary private
zsh hooks for prompt/command boundaries, exit status and OSC7 directory reports. It defaults to
false and applies only to new sessions, including profiles. Interactive zsh launches using only
`-i`, `-l`, `-li`, or `-il` arguments are supported. Orlo preserves usual startup files, user hooks
and ZDOTDIR changes and never rewrites shell configuration; private files are removed on close.
Other configured arguments and shells run normally without these hooks. Genuine OSC133 markers
are recognized with injection off too; without markers, the terminal stays fully usable and no
commands are guessed from prompt text. User customizations that disable hooks can prevent reports.

After a command completes, **Previous / Next Command** review its retained output while the
shell continues. **Copy Command Output** copies the reviewed record (the latest when viewing
live output); **Copy Last Output** always copies the newest completed record. Prompts/input are
excluded, and navigation/copy never send input or execute output. **Live Output** returns to the
running screen. These are terminal toolbar controls
and Tools/palette commands. Records keep up to 32 commands and 1 MiB of text in memory and are
discarded with the session. Output is the rendered retained text, with terminal formatting
removed; scrollback limits can omit its beginning, and trailing empty rows are omitted.

The Git icon at the right of the status bar, the branch name beside it, or `⌃⇧G` toggles the Git
side bar on the right; the terminal can stay open below. Set Up Git initializes a repository in
the shown folder without setting an identity or a remote. The staged and unstaged lists cover
the repositories of the project folders and open files, each file with its index status, then
its working-tree status. Side bar badges `S`, `M`, `?` and `!` mean staged, modified, untracked
and conflicted; a folder's badge includes its contents. Up/Down selects a file and shows its
saved diff, Enter opens it, Space stages and Shift-Space unstages. Commit opens local Subject,
Body and Coauthors fields and commits staged contents only; it never stages files automatically.
Enter submits from Subject; in Body and Coauthors it adds a newline, and `⌘Enter` submits.
Coauthors are optional, one `Name <email>` per line. Orlo validates identities, deduplicates
matching coauthors across these fields and the message footer, and inserts canonical
`Co-authored-by: Name <email>` lines alongside existing trailers. Names and emails match exactly,
apart from spacing in names; the first spelling and order are retained. Git's read-only footer
parser recognizes the original message's configured separators and aliases; unrelated URL prose,
sign-offs, folded metadata and legacy conflict notes keep their text. New coauthors go above scissors and patch cutoffs,
preserving the ignored tail. Assisted coauthors bypass `trailer.*` insertion/command rewriting;
use manual message text or commit hooks for that policy. Unicode names and multiline bodies are
supported. This composition help is local and does not generate text with AI.

Amend HEAD… is a separate Git panel, Git menu and command palette action. It prefills HEAD's
subject and body (including existing trailers) and replaces that commit with the staged index
and edited message. It also works with no staged changes, for message-only amendments; a
repository with no commits cannot amend. Ordinary Commit Staged… remains the default.
Amend preserves the previous author and author date. Both forms have optional Author name and
Author email fields: leave both blank to keep Git's normal author choice, or enter a complete
pair to change this commit's author explicitly. This does not change the committer or Git config.
Repository, branch or HEAD changes invalidate an open form; reopen it to review the new target.
Cancelled forms and Git failures retain the text in memory for retry in that window. Each mode
keeps one repository's draft until success or composing in another repository; quitting does
not save drafts. Text edited while Git runs is retained separately from the submitted snapshot.
Git-configured identity, hooks and signing run normally, and their errors appear in the form.
Failed commits retain staged and working-tree contents; the form keeps the message and author
fields for retry. Commit hooks retain their normal ability to enforce policy and edit files.

Create Stash… and Stashes… are in the Git sidebar, Git > Stashes and command palette.
Create stashes saved tracked changes, including the staged index; Include untracked files is
an explicit option and never includes ignored files. The optional message identifies the stash.
Stashes lists each entry with its object ID and offers a read-only Preview, Apply, Pop and
confirmed Drop. Preview includes untracked changes, summarizes binary files and stops at
2 MiB with a truncation note. Apply keeps the stash; Pop removes it only after successful
application. Their Restore Index variants also restore staged/unstaged separation, including
partial staging. Git conflicts keep the exact stash and the original Git error. A conflicted
Apply or Pop opens the existing Git sidebar/diff and first conflicted source, with the caret at
its first complete conflict block. Use Git: Next Conflict and Accept Ours, Accept Theirs or Accept
Both; these are normal undoable buffer edits. Save, then stage the resolved file. Newer panels,
file/Git selections, shared edits or IME composition prevent automatic navigation/reloading;
open the conflicted file from Git after finishing those changes. Binary/deleted/large files
retain the existing patch or file-open behavior. Resolve conflicts before retrying a stash.
Drop permanently removes the selected stash after confirmation without applying it. Deletion
rechecks the selected entry while holding Git's stash-reference lock: a competing terminal
push/drop makes the selection stale or waits/fails on that lock, rather than deleting another
entry. Drop and Pop currently require SHA-1 object storage and the files reference backend;
unsupported formats refuse before Pop applies anything. Apply remains available through Git.
If Pop applies changes but removal fails, its error says so: refresh Stashes before retrying
and do not apply the same changes again. A storage failure during removal can leave a partial
ref/reflog update; the error includes the selected object ID for inspection.

Changing the stash stack, repository or HEAD invalidates an open stash selection/form; reopen
Stashes to review the current entries. Worktree changes refuse unsaved repository files in any
window, including read-only working copies, and wait for file writes/save preparations and
other Git operations. Save or close unsaved documents first. Existing clean-file reload and
log-follow resume after the operation; background saves honor the repository write lease.
Stash errors appear in Git's status message. Cancel before submitting a form/action to leave
Git unchanged; a submitted operation runs to completion. No stash UI state is persisted.

Worktrees…, Create Worktree… and Prune Worktrees… are in the Git sidebar, Git > Worktrees and
command palette. Worktrees lists the path and branch, identifying main/current, locked and
missing registrations and Git's prunable state/reason. Open in New Window opens the selected
folder through normal workspace opening and keeps the current window, project and unsaved
documents. Create defaults to a new
branch at HEAD; the picker also offers existing local branches. Enter an absolute destination
whose parent folder already exists. Git refuses an occupied destination or a branch already
checked out elsewhere. Creation does not switch the current branch or open a window automatically;
choose the created folder in Worktrees, then Open in New Window.

Remove… names the selected folder in a confirmation and deletes only a clean linked worktree;
its branch remains. Main/current/locked worktrees and tracked or untracked changes are refused.
Unsaved and composing documents in any window, pending save preparations and file writes also
prevent removal. Open documents stay open; save or close edits before retrying. Prune Worktrees…
confirms only registrations Git marks prunable, with each native reason, and expires them
immediately (`--expire now`). Listing uses that same expiry policy; locked and ineligible
registrations stay. Pruning removes metadata, keeping branches and worktree folders. Repository,
HEAD, branch/registration or selected-folder changes invalidate a pending action. Existing
registrations that no longer point to their own
checkout require repair with Git before continuing. Reopen Worktrees after a stale refusal. Submitted operations
run to completion; cancelling a picker, form or confirmation beforehand changes nothing.
No worktree UI state is persisted and no force/unlock/reset option is provided. Git owns its
filesystem/ref protections; Orlo's revalidation is a preflight, not an atomic transaction with
independent terminal Git commands or filesystem changes.

Local Tags… and Create Local Tag… are in the Git sidebar, Git > Tags and the command palette.
Local Tags shows each local tag's name, lightweight/annotated type, target object and annotation
message. The selected tag's full metadata wraps in a scrollable detail area. Choose a row to confirm deletion of that tag only. Create Local Tag first asks for a
commit revision (HEAD by default), resolves it, then asks for a name and optional message at the
shown commit ID. Leave the message blank for a lightweight tag; enter a message for an unsigned
annotated tag. Names follow Git's tag rules; duplicate names are refused without replacement.
These commands explicitly disable tag signing and editor prompts, including when `tag.gpgSign`
is enabled. Ordinary commit signing continues to follow Git configuration. Use terminal Git
for signed tags.

Creation retains the resolved commit and repository identity while the naming form is open;
changed HEAD/repository/project contexts refuse with an error. Deletion checks the selected raw
tag object while holding its reference lock, so a replacement annotation at the same commit is
also protected. Refresh Local Tags after a stale refusal. Symbolic tag refs and deletion in
native ref/object formats or extensions unsupported by the bundled libgit2 (reftable/SHA-256/partialClone) refuse safely;
manage those with terminal Git. All tag operations run in the background and show Git errors
in the status area. They never contact remotes, push tags, switch branches or change the index
or source files. Tag commands block implicit fetching and transport protocols; missing promisor
objects report a local error and must be fetched explicitly. A submitted operation runs to completion; cancel a form or confirmation first
to leave tags unchanged. No tag UI state is persisted.

Branch lists local branches and offers Create, Rename and Delete.
Create makes a branch at HEAD without switching; Delete asks for confirmation and uses Git's
merged-branch check, protecting unmerged commits and branches checked out in any worktree.

The diff shows the two versions the patch compares side by side, aligned as in Compare Files.
The working tree side shows the index's text against the saved file, as Git compares it after
the line endings and filters its attributes ask for; an untracked file has nothing on the left
and a deleted one nothing on the right. The staged side shows HEAD's text
against the index's (for a staged rename, the old path at HEAD). Both sides are read-only,
scroll as one and fold alike, with red and green line and within-line backgrounds. Binary,
non-UTF-8, symbolic-link and conflicted files, and a side over 10 MiB, show the raw patch
instead; the label above it says why.

In a text diff, place the caret on a changed line or select lines, then use Stage Lines.
Choose added lines on the right or deleted lines on the left; each can be staged independently.
Stage Hunk stages the change at the caret (or changes touched by the selection), and Unstage
Hunk does the same on the staged side. These operations change only the index. If the saved
file or index has changed since the diff was loaded, refresh and select the change again.
When staging a line alone would join it to a line without a final newline, use the whole hunk.

Git > File History… (also in the Command Palette) lists commits for the active file or
selected Git sidebar file, including its earlier names. Choose a commit to open its changes in
a read-only tab; Older commits… continues past each 100 commits. This is separate from File's
saved-version history. Blame File opens a read-only annotation of saved contents with each
line's author, local date and time, and commit ID; uncommitted lines are marked Working tree.
Save unsaved edits before requesting blame. Binary, non-UTF-8 files and files over 10 MiB
cannot be annotated.

Git commands run off the UI thread and show their errors in the panel. Staging reads the saved
file and refuses one with unsaved edits; switching branches refuses unsaved buffers in that
repository. Stash and worktree operations remain explicit; Orlo never forces a checkout.
There is no general discard-saved-files command.

**Remote Git is explicit.** Fetch, Pull, Push and Remotes & Upstream are in the sidebar,
the Git menu and the Command Palette. Opening a project, refreshing status, listing remotes
or changing their configuration never contacts a remote. Fetch shows one configured remote's
URLs and fetch refspecs in a wrapped confirmation, then updates local tracking refs without
changing checked-out files. Pull confirms the current branch, upstream, remote URL and exact remote ref, then uses
fast-forward-only. It overrides merge/rebase preferences for this invocation and refuses
divergent history without rebasing or creating a merge commit; use terminal Git to choose how
to integrate it. Detached or unborn branches must be switched/committed first.

Push shows the current branch, upstream and selected remote URLs, asks for the destination
branch, and can optionally set that destination as upstream. A wrapped confirmation shows the
complete source/destination refs and URLs before dispatch. It uses an explicit current-branch
refspec, never force or mirror, and does not delete remote branches. The default destination is the current
upstream's remote branch for that remote (including custom tracking-ref mappings), otherwise
the current branch name. Remotes & Upstream lists fetch and push URLs, adds/removes remotes,
and replaces a remote's URLs with one fetch URL and an optional push URL (blank uses the fetch URL). Removing a remote removes local tracking refs,
not branches on the remote. Set Upstream chooses an already fetched local remote-tracking ref;
Clear Upstream removes current-branch tracking. Configuration edits are repository-local;
settings inherited from included/global config may need editing at their original source.

Git keeps its configured credential helpers, SSH configuration, signing and hooks. Terminal
credential prompts are disabled, so configure a working helper or authenticate through
terminal Git if needed.
Operations run off the UI thread with progress and bounded error text. Cancel a picker/form
or confirmation before submitting; submitted Git processes finish normally, even if the
originating project/window closes. No automatic fetch, clone UI or hosted PR client is provided.

Pull refuses unsaved, read-only or composing repository buffers in any window, pending save
preparations and overlapping writes. Fetch/Push preserve dirty source buffers and allow source
saves while holding shared Git metadata leases. All remote/config operations guard unsaved
Git config documents, including empty or absent global, XDG and system roots and optional
missing includes outside the repository. Remote controls require Git 2.42+ to discover these
roots using native environment rules. Ambiguous root paths containing newlines are refused;
ordinary local Git actions retain their existing requirements.
Declared conditional include targets are guarded conservatively even when currently inactive;
relative/tilde paths and dangling aliases resolve to their future save destinations. Leases outlive
window clearing, and stale repository/branch/URL/upstream forms must be reopened. External Git
processes can still race between identity checks and execution; Git's native locks and errors
apply. A two-field URL edit can partially succeed if the second config write fails; its error
identifies the completed fetch-URL change.

Green, blue and red gutter markers show additions, modifications and deletions against the Git
index, or against the saved file outside Git. Click a marker or use Diff: Show Change (`⌘K ⌘H`)
for its old and new text; `⌥↓` / `⌥↑` moves between changes. Revert Hunk edits the buffer as one
undoable step (save separately); an edit or a refreshed baseline makes an old review stale before
it can revert newer work. Over 500 line edits become one hunk. Git: Next Conflict moves to a
merge block, including diff3 blocks; Accept Ours, Accept Theirs and Accept Both replace it as an
undoable edit. Save the result, then Stage File: remaining conflict markers prevent staging.

The FOLDERS heading shows New File and New Folder under the pointer, created in the selected
folder or the first project folder; a window without folders offers Open Folder.

## Trusted folders

A folder you download can bring programs that run as soon as it is opened: a repository's own
`.git/config` can name filter commands that `git status` runs, a language server can build the
project (rust-analyzer runs `build.rs` and proc macros), and a project can define tasks. Orlo runs
these only in folders you trust. Trusting a folder covers the folders in it, and the list is kept,
readable by you alone, in `~/Library/Application Support/Orlo/Trusted Folders.txt`.

- **Git** runs as before in repositories whose own settings run nothing, which is almost all of
  them. One whose `.git/config` defines a filter (git-crypt, nbstripout, or a hostile one) waits
  in the Git sidebar with a **Trust Folder** button. Orlo never runs a repository's
  `core.fsmonitor` command or a `textconv` driver, and fetch, pull and push allow only the
  https, http, ssh, git and file transports.
- **Language servers** wait until the folder holding the project is trusted; the status bar
  shows **Language server waiting · Trust Folder**.
- **Tasks** a project defines in `.orlo-tasks.json` show their command and ask before their first
  run. Your own tasks and package tasks run as before.
- **Project > Trust Open Folders…** trusts every open folder at once.

## Distraction-free mode

View > Enter Distraction Free Mode (`⌃⇧⌘F`) shows the text alone: by default in macOS full screen,
without the side bar, tabs, status bar, minimap, panel or line numbers, wrapped at 80 columns in the
middle of the screen. The `distraction_free` setting chooses each of these. Inside the mode the
View toggles show the parts you want for that stay; leaving it (the same command, or leaving full
screen with the green button or `⌃⌘F`) brings the window back exactly as it was before, out of the
full screen the mode entered. A view's own Word Wrap and Word Wrap Column choices still win. The
session keeps the mode and the window before it.

## Project index and navigation

Goto Symbol, completion, definitions and the outline share the active buffer's index entry,
including unsaved edits. Parsing runs on the highlighting thread without a line limit, using
checkpoints every 1,024 lines. Edits are reparsed after a 300 ms pause, or immediately for
navigation. Goto Symbol in Project (`cmd-shift-r`) searches the window's indexed files and open
buffers, including untitled buffers, with previews; Escape restores the original view.

The folder index respects gitignore and `index_exclude_patterns`, skips files over 4 MB and
stops at 100,000 files; indexing the Go toolchain's 15,523 source files keeps about 405 MB
allocated. The status bar shows progress or a limited index. `index_files: false`
turns folder indexing off; open-buffer symbols and local completion remain available.

Goto Definition (`f12`, `cmd-alt-down`) uses a nonempty language-server answer first, then the
index. One indexed definition opens directly; several open a preview list ordered by locality;
none reports that no definition was found, including whether indexing is incomplete.
Goto Definition to Side opens in the next group, creating two columns when needed.
`shift-f12` always searches lexical whole-word text matches, labels the results as lexical and
marks indexed definitions `def`. Semantic references are in Goto > Language Server and the
palette. With `show_definitions` (default true), resting on a code identifier for 500 ms shows
signatures and doc comments; click opens the definition, Command-click opens it to the side.

Completion adds indexed symbols from the same syntax family (JavaScript/JSX and TypeScript/TSX
share one family). Indexed Plain Text files supply words to Plain Text completion.
Ranking combines fuzzy and prefix matching with nearby/frequent words,
project frequency, scope, receiver/member and call context, symbol locality and the window's last
50 committed completions. Comments suppress keywords and prefer prose; paths appear in strings.
Changing syntax or the index context closes an obsolete list. Completion works without a server.

View > Show Outline opens a panel to the right of the editor groups. Its visibility is kept per
window in the session. Symbols nest by indentation; the symbol at or above the primary caret
stays highlighted and visible. Click a row to jump and focus the editor. A stale entry shows
“Parsing…” after 300 ms; a fresh empty entry shows “No symbols”.

Goto > Switch File (`cmd-alt-up`) cycles files in the same folder with the same stem and a
different extension. Related Files… lists counterparts, conventional test/source names,
lexical includes/imports and quoted paths that resolve beside the current file or under a project
folder. Missing files are omitted; the buffer scan reads at most its first 10,000 lines.

## Language intelligence

Orlo bundles Tree-sitter parsing and highlighting for TypeScript/TSX, Python, JavaScript/JSX,
Java, C#, PHP, Bash, C++, HCL/Terraform, Go, Kotlin, Ruby, Swift, Rust and C. Module extensions,
headers, Kotlin scripts, Ruby build files and Bash startup files are detected too. Parsing runs
on the highlight worker and reuses trees after edits. Installed Sublime grammars override the
bundled parser. Other languages and documents above 16 MiB use syntect where a grammar exists,
otherwise plain text; individual lines above 20,000 bytes draw plain tree colors while retaining
parser context for later lines. HolyC has a bundled Sublime grammar for `.HC`, `.hc` and `.HH`
files; FreeBASIC has one for `.bas` and `.bi` files. Both appear under View > Syntax.
See the [language setup guide](docs/user-guide/languages.md)
for extensions, server commands and prerequisites.

Languages are data: each is a folder with a `language.json` manifest, under `assets/languages/`
(compiled into Orlo) or `~/Library/Application Support/Orlo/Languages/<id>/`, where a folder
with a bundled language's id replaces it whole. A folder can bring a `.sublime-syntax`, a
Tree-sitter grammar compiled to WebAssembly with its highlight and fold queries, and a language
server and formatter command. WebAssembly grammars run in a wasmtime sandbox, so one that
crashes is reported and dropped while Orlo keeps going. Pony (`.pony`, `pony-lsp`) and Nim
(`.nim`, `.nims`, `.nimble`, `nimlangserver`, formatted with `nph`) are bundled this way with
WebAssembly grammars; Rux (`.rux`, except files starting with `<`, which are Rux's UI language)
has a bundled Sublime grammar. **Reload Customizations**
reloads the folders, and their errors appear in its alert. To write one, see
[Adding a language](docs/user-guide/adding-a-language.md); `Orlo --check-language <folder>`
checks a folder without opening a window.

Inline diagnostic messages are off by default. Enable `"inline_diagnostics": true` in Preferences
or file/project overrides; `"inline_diagnostics_severity": "warning"` sets the inclusive
threshold (`error`, `warning`, `information`, `hint`). One message appears after each visible
line end when space permits. Long messages are elided; **Goto > Language Server > Diagnostics**
retains full detail for every admitted diagnostic across open tabs. See the
[language guide](docs/user-guide/languages.md).

Local language servers add diagnostics with underlines and gutter marks, semantic coloring,
completion, definitions, references, documentation, symbol rename, document formatting and code
actions. Goto > Language Server and `LSP:` entries in the palette expose the commands:

- `ctrl-space`: the completion popup (Edit > Show Completions), in every buffer. Where a server
  serves the buffer it is asked too, and its text and snippet items join the list when they arrive,
  each row with the server's kind and detail; Goto > Language Server > Complete asks the same
  way. Typing never asks a server. Enter or Tab applies the selected item, a server's as one undo
  step. `ctrl-space` says nothing when the server cannot answer; the menu command says why.
  Server snippets use the existing editable fields, mirrors and Tab / Shift-Tab navigation.
  Choice fields open the same choice list as local snippets. The main expansion and additional
  edits share one undo step at the primary caret; local multicaret snippets are unchanged.
  Invalid snippets refuse the whole completion, and cancelled or stale resolutions are discarded.
- `f12` / `cmd-alt-down`: semantic definition, falling back to indexed definitions when unavailable or empty. Goto Definition to Side (palette) opens in the next group.
- `shift-f12`: lexical whole-word references with previews and `def` markers; `ctrl-k ctrl-i`: documentation.
- `f2`: rename; `alt-shift-f`: format the document; `ctrl-.`: code actions.
- **LSP: Format Selection** (Goto > Language Server > Format Selection) formats the primary
  nonempty selection when the server supports range formatting. It remains an undoable, unsaved
  edit and uses the same stale-result and read-only protections as document formatting.
- Goto > Language Server > Signature Help (palette: **LSP: Signature Help**) shows the active
  signature and highlights its current parameter. Servers can also open and update help as you
  type their declared trigger characters. Use Up and Down to select an overload and Escape to
  dismiss it. Moving the caret, scrolling, resizing or leaving the view dismisses the popup.
  Explicit invocation starts a fresh request at the current caret. Restart Language Servers also dismisses
  displayed help and cancels pending help requests.
- Show Diagnostics lists messages and jumps to their ranges. Restart Language Servers retries a
  failed server. The status bar shows the server state and error/warning counts.

Install the servers separately and put their executables on Orlo's PATH, or configure absolute
paths in Preferences. The [setup guide](docs/user-guide/languages.md) lists all defaults.
JavaScript and TypeScript share one session per project root, as do C and C++. Terraform dialects
use `terraform-ls`; generic HCL has a separate, initially disabled server entry because the
Terraform server does not support it. Go semantic tokens are enabled by default. Each server's
capabilities determine which commands and semantic coloring are available; Python's default
server has no semantic-token provider.

```jsonc
{
  "lsp_enabled": true,
  "language_servers": {
    "rust": {"command": ["/absolute/path/rust-analyzer"], "initialization_options": {}},
    "go": {"command": ["gopls"], "initialization_options": {"semanticTokens": true}},
    "typescript": {"command": ["typescript-language-server", "--stdio"], "initialization_options": {}},
    "python": {"command": ["pylsp"], "initialization_options": {}}
  }
}
```

Omitted server entries use their defaults; an empty `command` disables that family. Commands
are argument arrays without shell expansion. These are application settings. Saving Preferences
restarts sessions when their configuration changes. Opening/changing/saving/closing buffers
synchronizes their unsaved text, including shared views and windows. Opening sends the whole
text; each later change goes as ranged edits from the exact text the server last received, so no
edit between syncs is lost. A server that asks for full sync gets the whole text each time, and
so does one whose ranges would outweigh the text or whose text has a `\r`. Positions use the encoding agreed with each
server (UTF-8, UTF-16 or UTF-32) and the protocol's lines, which end at a lone `\r` too. Missing
servers leave editing and saving available. Start Orlo from a shell if Finder's PATH does not include your tools.

Server edits validate the entire batch before changing text, reject stale/overlapping edits and
preserve one undo step per buffer. Cross-file edits open unsaved tabs; save them explicitly.
Completion does not execute snippet placeholders or attached commands. Code actions may resolve
an edit or execute an explicitly chosen server command; combined actions apply the edit first.
Server-initiated `workspace/applyEdit`, file creation/rename/deletion, annotated edits and pull
diagnostics are unsupported. Servers requiring those report failure. LSP documents are limited
to 8 MiB, messages to 16 MiB, and requests to ten seconds.

## Settings

Orlo > Settings… > Settings (`cmd-,`, or "Preferences: Settings" in the Command Palette) opens
`~/Library/Application Support/Orlo/Preferences.sublime-settings`. Until you save it, the tab
shows a template listing every key with its default. The file is JSON with Sublime's comments and
trailing commas, and keys Orlo does not know are ignored, so a Sublime preferences file can be
pasted in:

```jsonc
{
	"font_face": "Menlo",
	"font_size": 12,
	"tab_size": 4,                     // 1–16; tabs are drawn at these stops
	"translate_tabs_to_spaces": true,  // false: tab and indent insert a tab character
	"word_wrap": "auto",               // true, false, or "auto": wrap everything but source code
	"wrap_width": 0,                   // 0 wraps at the view's width; N at most N columns
	"rulers": [],                      // e.g. [80, 100]
	"show_minimap": true,              // Orlo's own key
	"theme": "auto",                   // follows macOS; or "Default.sublime-theme", "Default Dark.sublime-theme"
	"trim_trailing_white_space_on_save": false,   // true or "all"; or "not_on_caret": spare the caret's line
	"remove_trailing_blank_lines_on_save": false, // Orlo's own key
	"ensure_newline_at_eof_on_save": false,
	"animation_enabled": true,         // the side bar slides, Page Up/Down glide
	"fold_buttons": true,              // ▾ in the gutter beside foldable lines, ▸ beside folded ones
	"fade_fold_buttons": true,         // ▾ only while the pointer is over the gutter
	"draw_white_space": "selection",   // "none", "selection", "trailing", "all", or an array of them
	"drag_text": true,                 // drag selected text to move it (option at the release copies)
	"auto_complete": true,             // the completion list opens as a word is typed (ctrl-space opens it any time)
	"auto_complete_min_chars": 2,      // Orlo's key: the letters typed before it opens
	"spell_check": false,             // F6 overrides this for the current view
	"spelling_selector": "markup.raw, source string.quoted - punctuation - meta.preprocessor.include, source comment - source comment.block.preprocessor, -(constant, entity, keyword, punctuation, storage, support, variable, markup.underline.link, meta.tag)",
	"distraction_free": {              // Orlo's own key: what distraction-free mode shows
		"full_screen": true, "show_sidebar": false, "show_tabs": false, "show_status_bar": false,
		"show_minimap": false, "show_panel": false, "line_numbers": false,
		"word_wrap": true, "wrap_width": 80, "draw_centered": true,
	},
}
```

The three `_on_save` settings tidy a buffer as it is saved, in this order: trailing spaces and
tabs come off every line (`"not_on_caret"` spares a line whose white space has a caret of the
saving view in it or at its end; a no-break or ideographic space is text and stays), blank lines
at the end go (the last line's break stays), and a final line break is added. An empty document
stays empty. Save, Save As, Save All and the saves before closing run it once per buffer, as one
undo step in the saving view, after the file name is chosen; if the write fails the edit stays,
the tab stays unsaved and Undo takes it back. Session autosave never runs it, and line endings
and the byte order mark are untouched. With trimming off, Markdown's two-space line breaks stay.

`fold_buttons` draws a ▾ in the gutter beside a line that heads a foldable block and a ▸ beside a
folded one; `fade_fold_buttons` shows the ▾ only while the pointer is over the gutter (the ▸ always
shows). `draw_white_space` is Sublime's: `"none"`; `"selection"` (the default), the white space
inside selections; `"trailing"`, the white space after a line's last character that is not white
space; `"all"`, every space `·`, tab `→` and other Unicode space `°` and a `¬` at the end of every
line that has a line break; or an array of these, which marks their union (`["selection",
"trailing"]`). `drag_text` lets a press on selected text drag it (see Keys). `auto_complete` opens
the completion list as a word is typed, once the word before the caret has
`auto_complete_min_chars` characters (Orlo's own key, 2 by default); `ctrl-space` opens it at any
time (see Keys). These six are application-wide, as `theme` is: a project's `settings` or a scoped
override that names one is refused with a warning.

Unusual Unicode characters receive small warning overlays by default: amber where readable,
black or white otherwise. Set
`unicode_warnings` or replace `unicode_warning_characters` in Preferences; both support
per-syntax overrides, but not project files, so a folder cannot hide the bidi controls and
invisible characters it brings ("Trojan Source"). **Edit > Character Info** explains configured warnings at the caret,
even with marks disabled. Emoji joiners, variation selectors and combining marks are excluded
by default. Overlays preserve document text and shaped geometry. See the
[exact defaults and configuration](docs/user-guide/customization.md#unusual-unicode-warnings).

Saving the file, in Orlo or in another editor, applies it to every window at once. A file that
does not parse shows the error with its line and column and leaves the current settings in force,
and a `font_face` that is not installed is named in an alert while the previous font stays.
Deleting the file keeps the settings in force. Orlo writes it to a temporary file and renames it
into place. View > Show/Hide Minimap, View > Show White Space, View > Ruler and the Font sizes
override the file in their window, and View > Word Wrap in its view, until it is next saved; Font >
Reset Size goes back to the file's size. The status bar shows "Spaces: N" or "Tab Size: N".

## Session

Opt-in `"file_autosave": true` saves eligible existing files after two seconds without edits
(configurable with positive whole `"file_autosave_delay_ms"`, default `2000`),
using ordinary formatting/cleanup and safe writes. It defaults off, defers during shared IME
composition or snippet fields, and protects externally changed files. Configure it through
Settings, including project/file overrides. See [automatic file saves](docs/user-guide/text-essentials.md#automatic-file-saves)
for ownership, failure and close behavior. It is separate from session recovery and file history.

Orlo keeps `~/Library/Application Support/Orlo/Session.json`, and the next launch reopens every
window where it was, on the display it was on: its folders (`folders`, with each one's name and
exclude patterns) and expanded folders, its project and workspace files, side bar width and
OPEN FILES, layout, the files in each group with their carets, scroll position, bookmarks, word
wrap and wrap column choices, valid per-view folds and pinned or preview state, the window's View toggles, scroll sync,
panel places and distraction-free mode, and the recent files, folders and projects. A session
from before keeps its one `folder`, which becomes the only root; the first root is still written
as `folder`, so an older Orlo opens it. A project file the session names that is gone or cannot be
read leaves the session's folders in place and is not written over. Files deleted in between are skipped, files on a drive that is not mounted stay in the
session for next time, and a window whose display is gone opens on the main one. Find Results is
not restored.

Unsaved changes are kept without asking (Sublime's hot exit): edited and untitled buffers come
back dirty, with their line endings, and a file changed on disk meanwhile offers a reload. The
session is written a few seconds after edits settle (off the UI thread, and at least every 30
seconds while typing goes on), when a window closes, and at Quit. `kill` (SIGTERM), Ctrl-C in
the Terminal that started Orlo (SIGINT) and closing that Terminal (SIGHUP) write it as Quit does
before Orlo exits, so only a crash or `kill -9` loses those last seconds. A second signal, or ten
seconds with Orlo still running after the first (a stuck disk), ends Orlo at once, as the signal
would have without Orlo's handler. A write that fails is tried again after 2 s, then 4, 8 … up to
64 s; after three failures in a row the status bar says SESSION NOT SAVED until one succeeds. If
Quit cannot write it within two seconds, Orlo says so and stays open unless told to quit anyway;
a file opened with `Orlo <file>` while that alert is up still opens. **Unsaved text is
stored as plain text** in the `Session` folder next to `Session.json`, one file per buffer; the
folder is readable by you alone (0700, the files 0600). With no home folder at all, the support
folder is `Orlo-<your uid>` in the temporary folder, made private to you.

Closing the last window is not Quit: as in Sublime 4215 on macOS, it asks about unsaved buffers
(Save All, Discard All, Cancel), then Orlo quits; Discard All loses untitled drafts. Quit (`cmd-q`,
the Dock, logging out) keeps them.

A session file that is not valid JSON, has a wrong value or misses an unsaved text is kept as
`Session.json.<YYYYMMDD-HHMMSS>.bad` (UTC), with its unsaved texts in
`Session.<YYYYMMDD-HHMMSS>.bad/` beside it, and Orlo opens what it could read (a wrong value
falls back to its default). Unsaved texts the session does not name are moved into such a
folder too, never deleted. Orlo says when a launch has kept a snapshot, and where, with a Show in
Finder button. The newest five snapshots are kept, in the order the filesystem says they were
made (a clock set back does not reorder them), a folder of texts alone counting as one; an older
one goes, unless it holds unsaved texts and is not yet 30 days old. The snapshot just made is
never removed.

A session file Orlo cannot read at all (its permissions, say) is not written over: Orlo says so,
and File › Session Could Not Be Read… says it again while it lasts. Until you choose Start a New
Session (which sets the file aside the same way) or it can be read again, autosave writes
nothing; Quit, the Dock's Quit, logging out and the signals above move the file aside, byte for
byte, as a snapshot, and then save this run's session, unsaved changes included.

## File watching

While a window is in front, Orlo checks its open files and expanded folders every half second,
and at once when you switch back to it. A file changed elsewhere reloads in place, keeping carets,
bookmarks and scroll, and the reload can be undone. If the tab has unsaved changes, Orlo asks
first: Reload, Always Ignore (stop watching that tab) or Cancel (ask again on the next change). A
file deleted or renamed elsewhere keeps its tab with a red dot, and `cmd-s` then opens Save As in
its old folder; closing that tab asks "The file … was deleted. Do you want to save it?", as
Sublime does, and Save All shows the Save As panels one after another; a Save As asked for while
a Save panel is open waits for it to close, then carries on (closing the tab, quitting). The side
bar shows files appearing, disappearing and being renamed, and dims what each folder's own
`.gitignore` lists as it changes. The status bar's git branch follows checkouts made elsewhere; it shows the
short hash of a detached HEAD, as Sublime does, and works in worktrees, submodules and folders
inside a repository. Files are read and folders listed in the background.

**File > Follow Log** explicitly follows a saved, clean file in every view/window of that
file. It is off by default and is never restored with a session. While following, source
text cannot be edited, formatted, undone or saved; navigation, selection, copy, Print and
Send Selection still work. Pending saves, active shared IME composition and write operations
must finish first. **Stop Following Log** or the persistent **FOLLOW · Stop** status control
ends the mode in all views. Incoming text changes clear text Undo history; ordinary Revert
keeps its existing undo behavior.

The initiating view goes to the end. A single caret at the end follows new output until you
select text or scroll away. Scrolling back to the bottom resumes the paused tail caret if you
have not moved it or changed the selection during that pause. An unrelated caret, selection,
multiple carets or virtual caret keeps its inspection state; choose Follow Log again or
navigate to the end to resume there. Other views keep their own position. Save or discard
unsaved edits before starting Follow Log; a refused start leaves those edits and their
undo/recovery state intact.

Append, truncation and replacement at the same path are followed, including a temporary
missing-file interval during rotation. Reads and
comparisons run in the background every half second, within the existing 100 MiB file limit.
Incomplete Unicode/CJK characters and BOMs wait for more bytes, using the disk encoding with
external BOM precedence. Malformed data/read errors pause with a reason in the status tooltip;
Stop and reload/reopen with the correct encoding to recover. Stopping with incomplete data
keeps source overwrite protection until a complete ordinary reload. Renaming the document or
closing its last view stops the worker; closing one shared view keeps the others following.


## Spell checking

View > Spell Check (`f6`) toggles red wavy underlines for the current view; `spell_check` defaults
to false. `spelling_selector` controls eligible scopes: its default checks prose, comments and
quoted strings while excluding code identifiers/keywords, links and tags. Tokens containing
digits or underscores, camelCase/PascalCase with inner capitals, URLs, e-mail addresses and paths
are excluded. These settings also support project and file overrides. View toggles and caches
are not saved with the session; sibling views can choose independently.

View > Next Misspelling / Previous Misspelling (`ctrl-f6` / `ctrl-shift-f6`) searches the whole
document and wraps once. Right-click a misspelled word or use Spelling Suggestions… in View or
the palette. A guess replaces the exact checked range as one ordinary undoable edit. Ignore
Word lasts for this app session across documents/windows; Learn Word adds to the system user
dictionary, and Spelling Suggestions… offers Unlearn Word for learned words. Dictionary actions
invalidate results across views; edits, caret/context changes and newer panels reject stale choices.

Native checks and dictionary work run on one spelling worker; syntax preparation stays on the
highlighting worker. Visible checks are bounded, so a word crossing their 256-character context
margin can remain unmarked. Whole-document commands check boundary-spanning words intact, with
no intentional word-length cutoff; memory and a single native call's duration scale with such a
word. An entered native call cannot be interrupted. Lines over 20,000 bytes use the same
starting-scope fallback as highlighting. Native checking, guesses, word/learned reads and
private-tag Ignore are tested against the real macOS checker; persistent Learn/Unlearn effects
are tested only against a fake.

## Word wrap

Lines wrap at the view's width the way Sublime breaks them: after spaces, tabs and `/ ( ) [ ] <
>`, with continuation rows indented like the line, and as many columns as Sublime fits in the same
width. A line of nothing but whitespace is cut at the width like any other unbroken run. The setting `word_wrap: "auto"` wraps everything
that is not source code (plain text and Markdown wrap, Rust does not); View > Word Wrap (or
"Word Wrap: Toggle" in the Command Palette) flips the current view only. Up and down move by row
and keep to where the caret is drawn, `cmd-left` / `cmd-right` go to a row's start and end first, then to the
line's, and `ctrl-a` / `ctrl-e` go straight to the line's start and end. The line number shows on
a line's first row, and the minimap wraps too.

View > Line Numbers > Absolute / Relative (palette: View: Line Numbers: Absolute / Relative)
chooses the gutter labels for this window. Absolute is the default; set
`"line_number_mode": "relative"` in Preferences or file/project overrides to change the default.
Relative shows the primary caret's physical source line as its absolute number and other lines
as their nonnegative source-line distance, including hidden folded lines. Each shared view
uses its own primary caret. Wrapped continuation and filler rows have no numbers; joined fold
rows keep their first source line's label. Go to Line still takes absolute source line numbers.
The choice leaves text, selections and clicks unchanged and respects distraction-free gutter
visibility. Window choices reset when Preferences change or the window is reopened.

View > Highlight Cursor Columns (palette: View: Highlight Cursor Columns) adds faint background
bands at each distinct selection-head display column across visible text rows. It starts off;
set `"cursor_column_highlight": true` in global, scoped or project preferences to enable it.
Each shared view follows its own carets, including virtual space, through tabs, Unicode,
wrapping, folds and scrolling. The window toggle resets when Preferences change or the window
reopens. The decoration leaves text, clicks, copy/save and print output unchanged.

The `wrap_width` setting (or a project's) wraps at most that many columns instead, and at the
view's width when the view is narrower; 0, the default, is the view's width. View > Word Wrap
Column sets it for the current view: the setting's, the view's width, or 70 to 120 columns.
Columns are display columns: a tab runs to its tab stop and an East Asian wide character or emoji
counts two. It is display wrapping only: the file's lines are unchanged, and rulers stay where they
are whatever the wrap column. Motion, selection, clicks, the minimap and scrolling go by the same
rows, and the text on top stays on top when the wrap column, the font size, the side bar, a dock
or the layout changes the width.

## File and folder comparison

Tools > Compare > Compare Files… / Compare Folders… (also in the palette and custom bindings;
no default launch shortcut) asks for the left and right existing paths. Cancelling, or a path
that is refused, keeps the current comparison. The transient Comparison panel uses the Changes panel's
dock placement and keeps normal tabs. Its read-only snapshots show full paths; `Disk`,
`Live buffer` or `Live buffer · unsaved`; and UTF-8, BOM and line endings. A unique open buffer
in any window supplies its unsaved text; independent buffers owning one path must be reconciled first.

The two sides line up row for row. Where a difference is shorter on one side, faint hatched
filler rows pad it at the difference's end, so every unchanged line sits level with its
partner. This holds when a line wraps to more rows on one side, or the panes have different
widths. Filler rows have no text or line number: carets and Up/Down step over them, a click on
one lands at the end of the line above, and copying and finding never include them. Scrolling
either side takes the other to the same row, not the same line. Folding a range on either side
folds the matching text on the other: unchanged text matches char for char, and a fold's edge
inside a difference keeps its line in the difference and its column, within the other side's
lines and their text; where the other side has no lines, it goes to the difference's place.

Previous Difference / Next Difference moves both viewers through red/green line and separated
within-line ranges, including empty/EOF labels. Format-only differences are labeled separately.
Apply Left to Right / Apply Right to Left changes the selected hunk in an ordinary destination
buffer, preserves its format and shared views, focuses it and reports "Applied to buffer — not
saved". It is one Undo/Redo step; disk changes only on ordinary Save, including its usual save
hooks and safe-write behavior. Refresh is required before another Apply.

Folders show sorted relative paths, hidden files included, without automatic gitignore/VCS
exclusions. Statuses are Different, Left only, Right only, Type conflict, Unsupported and Error;
Show Equal adds equal rows. Include globs and Exclude globs take semicolon-separated gitignore-style
patterns, with `!` exceptions (`src/;!src/skip.rs`); malformed patterns show an error. Filters
apply to both sides, show counts and never let an operation act on a hidden row. Select a supported
matched file to retain the folder list beside its pair. Refresh explicitly rescans the roots.

A supported clean one-sided regular file offers Copy File to Left/Right. Confirmation names the
source, new destination and missing parents. This creates a file on disk that Undo does not
remove; save a dirty source and Refresh first. Copy rechecks roots, source and absence, app-wide owners,
loads and write reservations; it uses pinned parents, private staging, native metadata copying
(including ACLs/security attributes) and exclusive publication. A raced destination is never
overwritten. Cancellation/failure removes only owned staging data and still-empty new parents;
changed/nonempty paths are preserved and reported. A crash can leave stages or parents.

Comparison reads cap each file at 100 MiB; inventories stop at 100,000 encountered entries across
both walks, counting directories/errors, with Limited / Incomplete and unknown absence reported.
Within-line detail caps at 20,000 graphemes per paired line, then uses coarse ranges; over 500
line edits can form one middle hunk. NUL-binary, invalid UTF-8, oversized files, symlinks and
special files cannot be text-applied or copied; matched binary/lossy folder files can have byte
Equal/Different status for inspection. No bulk sync/delete, tree copy, three-way merge, automatic
watching or persisted comparisons is provided.

Reads, walks, filters, diffs and Copy work run in the background with cooperative cancellation.
Apply rechecks exact reviewed bytes and both live owners; Copy retains live ownership guards
through publication. Unrelated ownership/window/reservation changes can conservatively require
a fresh review. Final Copy checks depend on filesystem stamp precision: a same-size external
rewrite preserving the complete stamp can evade them on coarse/network volumes. There is no
transaction/content CAS against arbitrary external processes, or added guarantee for a later
ordinary Save.

## Find in Files

`cmd-shift-f` opens the Find in Files panel under the window: Find, Where and Replace, with
toggles for regex, case and whole word (which change matching) and Show Context, Use Buffer,
Use gitignore and Preserve Case (which do not). `enter` (or Find) searches into the Find Results
tab; `cmd-alt-enter` (or Replace…) previews replacements in the results browser. `tab` and
`shift-tab` move between the fields.

The Where field takes folders, files, `<open folders>`, `<open files>` (this window's),
`<previous results>`, include globs such as `*.rs` and `-` exclude globs such as `-target/`,
separated by commas; empty means the open folder and the open files. Its `…` button (or
`alt-down` in it) adds folders or files from a chooser, a token, the side bar rows marked with
`⌘`-click, a recent Where value or a saved search's scope. Find > Find in Side Bar Selection… puts
the marked rows in Where. `<previous results>` is every file the window's last Find in Files had
a match in (a binary one too). Rules, in order:

1. A file comes from a folder walked, a file named, `<open files>` or `<previous results>`;
   each real file once (a link and its target, or a file named twice, are one).
2. An exclude glob matching the file or a folder above it leaves it out, whichever source named
   it; with include globs, the file or a folder above it must match one. An include naming a
   folder (`src/` or `src`) takes in everything under it. Globs with a `/` are read from the
   folder searched (for a file named alone, from its own folder).
3. Folders walked only: `.gitignore` files (nested ones too, with or without a `.git` folder;
   those above the searched folder up to its repository's root, none outside one) and your global
   git excludes apply while Use gitignore is on; binaries, images and `.git`, `.svn`, `.hg`
   folders are always skipped. Files named directly are never gitignored.

A glob that does not parse is an error under the panel and nothing is searched, so a typo cannot
widen the search. A Where that names nothing, or `<previous results>` before there are any, is an
error too.

Use Buffer (on by default) searches every file open in any Orlo window as it is in
the editor, unsaved changes included, also through a symlinked folder such as `/tmp`; off, every
file is read from disk. Show Context puts two lines around each match (off: matching lines
only). Find in Files searches each file's whole text, its line breaks as the editor holds them
(CRLF and CR files read as `\n`), so a query with a line break, `\n` or `(?s)` matches across
lines; a match is listed on every line it covers (the first ten), and Find Results outlines it
once. Results stream into a Find Results tab in Sublime's format while the status bar counts the
files; a results line over 1,000 characters is cut around its match, never its location. Double
click a match to open the file with the caret on it (a file name or context line opens at the
start of that line). Editing Find Results keeps the outlines on their matches, and undo brings
back the ones an edit removed. Find > Cancel Find in Files stops a search, and a new one appends
to the same tab; "File: New View into File" on Find Results gives a second Find Results view.
Files over 100 MB are listed as too large; a file that fails to search (a pattern that runs away
in it) says why instead of a count.

### The results browser and Replace in Files

Find > Search Results (and every Replace…) shows the window's last results in a panel along the
bottom: each file with its count, collapsible (`▾`), and under it each match with its line and
column, its line with the match marked and, for Replace, `→` the line with its replacement, plus
context lines with Show Context. Filter narrows the rows to files or lines containing its text;
it never includes or excludes anything. Keys in the list: `up`/`down` move, `enter` (or a double
click) opens the source with the whole match selected and the keys staying in the browser,
`space` toggles the row's checkbox, `left`/`right` collapse and expand, `tab` or `cmd-f` go to
Filter, `escape` closes; F4 and Shift-F4 step through the match rows listed, opening each. A
file changed since the search opens at the recorded line and column and the browser says the
location may be stale; a deleted one is only reported.

Replace… previews: nothing changes until Apply. Each match and file has a checkbox
(`☑`, `☐`, `◩` for some), and the counts say how many of how many are selected; Select All /
Select None, Expand / Collapse and Refresh (the same search again, a fresh preview) are beside
it. The preview is tied to the query, the replacement, the toggles, the Where and each file's
text as searched (an open buffer's version; a closed file's stamp and the hash of its bytes).
Apply then goes file by file:

- An open buffer (in any window), still at the previewed version: every selected replacement in
  one undo step, left unsaved for review. A file opened after the preview counts as unchanged
  while every view of it is unedited and was read from the file as previewed.
- A closed file: read again and compared with the preview (stamp and bytes); written by Orlo's
  safe save (a temporary file renamed over it, keeping its permissions, extended attributes,
  owner, byte order mark and line endings).
- Anything else is left alone and reported: **stale** (edited, changed on disk, closed or opened
  and edited since the preview: Refresh and apply again), **skipped** (malformed Unicode, binary,
  over 100 MB, being written by a pipeline run, or open in Orlo while Use Buffer is off and it has
  unsaved changes or Always Ignore), **failed** (read-only, a write error; the file stays as it
  was unless the write in place was cut short, which the report says), **excluded** or
  **unchanged**.

Cancel stops before the next file; files done stay done and the rest are reported as not
reached. A Replace Report tab lists every outcome. There is no global rollback: each changed
buffer has its own undo step, and files written on disk have no undo in Orlo, so keep them under
version control. A file can change between Orlo's last check and its rename; the window is the
write itself.

## Search and text tools

### Regular expressions

The `.*` toggle enables regular expressions in Find/Replace, Find in Files and the playground
(`cmd-alt-r` in a search field; `cmd-alt-c` case, `cmd-alt-w` whole word, `cmd-alt-a` Preserve
Case). Patterns use [Rust regex syntax](https://docs.rs/regex/latest/regex/#syntax) (Unicode
classes, `\b`, line anchors, named and numbered captures, `(?x)` comments, class set operations)
on regex-automata's linear-time engine. A pattern that also uses lookaround,
backreferences, atomic groups or conditionals runs on [fancy-regex
0.19](https://docs.rs/fancy-regex/0.19.2/fancy_regex/) instead, which adds `(?=…)`, `(?!…)`,
`(?<=…)`, `(?<!…)`, `\1`, `\k<name>`, `(?P=name)`, `(?>…)`, `(?(1)…|…)`, `\h` and `\Z` to the same
syntax. A lookbehind must have a fixed length, or be made of parts without lookaround,
backreferences or captures (`(?<=a+)` works, `(?<=(a)\1)` only at a fixed length); `\G` and `\K`
are refused. Both engines take `^`/`$` as line anchors and `\A`/`\z` as text anchors; `.` stops
at a line break unless `(?s)`. Patterns both engines accept give the same matches (a test runs a
corpus through both). A selection searched with In Selection bounds where a match may lie, while
anchors, `\b` and lookaround still see the text around it.

Lookaround patterns backtrack, so Orlo limits them: each start position may take 1,000,000
backtracking steps, past which the search stops and says on which line; a search on the UI thread
(Find Next, Replace All in a small buffer) stops after 2 s, one on another thread (find as you
type with lookaround, a buffer over 4M characters, each Find in Files file, the playground) after
60 s, and every one can be cancelled by typing on, closing the panel or Cancel. A stopped search
is an error under the panel, never "no matches"; Replace All then changes nothing. Invalid
patterns show their error and cannot replace text. Edit > Transform's Sort Lines by Pattern,
Process Duplicate Lines and Process Lines Matching (and their pipeline steps) use the same
dialect; substitution tables keep the Rust one.

Select text before opening Find/Replace, then enable `▦` (or `cmd-alt-s` inside the panel) to
search only those selections. The scope stays on that text as replacements change its length;
Find Next/Previous, Find All and both replacement commands use it. Switching buffers resets the
toggle. With no selected text the toggle stays off.

F4/Shift-F4 opens the next/previous Find in Files match, including another match on the same
line, and wraps at the ends; with the results browser open they step through its match rows
instead. These keys also work while editing the opened target file.

### Replacements

In regex mode a replacement takes `$1`, `$name`, `${name}` and `$$` (a literal `$`); use
braces before adjacent letters (`b${1}b`). A group the pattern does not have (`$3` with two, a
misspelt `${nmae}`, `$1a`) or an unclosed `${` is an error, reported before anything is replaced.
A group that took no part in the match expands to nothing, as an empty one does. `\U` and `\L`
upper or lower case what follows until `\E` (or the other), `\u` and `\l` the next character only
(`\u\L$1` capitalises), and `\n`, `\t`, `\\` (a backslash); any other backslash is itself. For a
literal backslash before `U`, `L`, `E`, `u`, `l`, `n` or `t`, write `\\`. Case changes use
Unicode's full mappings, which can change the length (`straße` → `STRASSE`; there is no
title-case mapping, so `ǆ` capitalises to `Ǆ`). Literal mode inserts the replacement exactly as
typed. Replace All is one undo step.

Preserve Case (the `Aa→` toggle) gives the replacement the case of what it replaces: all
lower case → lower case, all upper case (two cased letters or more) → upper case, capitalised
(first cased letter upper, the rest lower, a single capital included) → its first cased letter
upper, the rest as typed; mixed case, or no letters, leaves it as typed. With `\U`, `\L`, `\E`,
`\u` or `\l` in the replacement the explicit case wins and Preserve Case does nothing. Previews
(the results browser, the playground) and Apply use the same code, so what is previewed is what
is written.

### Fields, history and saved searches

The Find and Replace fields (in Find, Replace, Find in Files and the playground) take several
lines: a paste keeps its line breaks (CRLF and CR become `\n`; a whole-line copy's own break is
dropped), `ctrl-enter` inserts one, `enter` and `shift-enter` still find next and previous, and a
field grows to five rows, then scrolls. A selection of up to 16,384 characters seeds the Find
field, lines and all. Where stays one line.

Up/Down recall history in a field's first and last line and move between lines elsewhere;
`alt-down` (Find > Search History…) lists the field's history to pick from. Find, Replace and
Where each keep the latest 32 distinct entries submitted in this window (Find Next, Find All,
Replace, Replace All, a Find in Files run; not every keystroke), newest first; recalling one and
going past the newest returns the draft you were typing. History lives in memory only:
it is never written to disk, and Find > Clear Search History empties all three.

Find > Saved Searches keeps named searches in `~/Library/Application Support/Orlo/Saved
Searches.json`: Save Search… stores the open panel's query, replacement, toggles and (from Find in
Files) Where under a name, asking before replacing one of that name; Open Saved Search… puts one
back into Find, Replace or Find in Files (the one it suits) without running it; Rename… and
Delete… (which asks) manage them; Export… writes them all; Import… reads such a file, checks every
entry (JSON, version 1, names, patterns and replacements) before taking any, and gives a taken
name " 2", " 3" …. The file is written to a temporary file and renamed, and holds only what was
typed into the panels, never document text. To edit one, open it, change it and save it under its
name.

### Playground, extraction, marks and highlights

Find > Regex Playground opens a scratch "Regex Playground" tab for sample text (Find > Regex
Playground with a Copy of the Document copies the whole active document, which is never
changed) and a panel with Pattern and Replace fields. As you type, the sample is searched
on another thread after a 150 ms pause, a result for an older pattern or text being dropped:
matches are outlined, `enter`/`shift-enter` (or `▶`/`◀`) select the next and previous, the status
says "Match 3 of 12" or the error, and the capture inspector lists each group of the current
match by number and name with its position and text, "unmatched" for a group that took no part and
"empty" for one that matched nothing. Replacement shows what the current match becomes; Preview
All opens the whole sample replaced in a tab. Use in Find / Use in Replace / Use in Find in Files
take the pattern, replacement and toggles there (into the find history too); Save… and Load… use
saved searches.

Find > Extract copies the Find query's matches, or the lines they touch, to the clipboard (and
its history) or a new document, optionally with locations (`file:line:column: ` or `file:line: `,
1-based, columns in characters), from the active document (its selections with In Selection on)
or from the last Find in Files results (each file read as it was searched; if one changed since,
nothing is extracted until you refresh). Matches keep their order and their line breaks; empty
matches give nothing. Lines come out once each, however many matches they hold; a match ending
just after a line break does not take the next line, and an empty match gives its line.

Goto > Bookmarks > Bookmark Matching Lines bookmarks every line the Find query's matches touch
(the same lines as extraction), added to the bookmarks there are; Invert Bookmarks swaps them.
Copy, Cut, Delete, Keep Only and Paste to Bookmarked Lines act on the bookmarked lines: copying
puts them on the clipboard as whole lines (pasting puts them above the caret's line, as a line
copy does), deleting takes each line with its line break (the last line takes the break before
it), and pasting replaces each bookmarked line's text with the clipboard's, line by line when
the counts match. Each change is one undo step with one caret where the first line went, and undo
brings back the view's bookmarks with the text.

Find > Highlight marks text in five styles (red, orange, green, blue, purple, at a third so text
stays readable; selections and the find outline stay distinct): Highlight Matches takes the Find
query's matches when the panel is open, else every occurrence of the selected text (or of the
word at the caret, whole words, case as typed); Highlight Selection the selections. Next and
Previous Highlight move to a style's ranges, Copy Highlighted Text copies them one a line, Clear
Highlights clears a style or all. Highlights stay when the Find panel closes or its query changes.
They belong to the view (a new view of the file starts with them), follow edits in any view of
the file, go with their text when it is deleted and come back with undo, survive a reload, and are
kept in memory only: a session restore does not bring them back.

Set `"selection_occurrences": true` in Preferences > Settings to shade other literal,
case-sensitive matches of one nonempty single-line selection, up to 1,024 Unicode scalars.
Larger selections clear the overlay and start no worker. This defaults off and supports
project and file overrides. Matching runs on a cancellable worker with Find's existing search
deadline and left-to-right non-overlapping rule (`aa` in `aaaaa` matches at 0–2 and 2–4).
The overlay is local to each view, including read-only/viewer views; empty, multiple, multiline,
virtual or composing selections clear it. Find and the five manual styles stay independent.

View > Indentation and the Command Palette choose the active buffer's tab width and whether
future indentations insert spaces or tabs. All views of that buffer agree, and the session
keeps the choice. Changing font size, rulers, the minimap or preferences preserves explicit
buffer choices. Convert Indentation is the command that rewrites existing indentation.

Edit > Text and the palette offer Trim Trailing Whitespace and Remove Blank Lines. They process
selected lines when text is selected, otherwise the whole buffer, with one-step undo. Cleanup
recognizes spaces and tabs; nonbreaking and ideographic spaces remain text. Nothing is trimmed
on save unless the `trim_trailing_white_space_on_save`, `remove_trailing_blank_lines_on_save` or
`ensure_newline_at_eof_on_save` setting is on (see Settings).

## Columns and CSV/TSV

Edit > Columns and the palette have no key bindings (Sublime has none). A column is a *display*
column: a tab advances to the next tab stop, East Asian wide characters and emoji count 2,
combining marks and joiners count 0, and a line shorter than a column has empty space after its end.

Column Editor… opens a form. With Mode set to Text it inserts the text at every selection; with
Numbers it inserts Initial, Initial + Increment, … (integers, 64-bit; Repeat uses each value that many times
first), in base 10, 16, 8 or 2 (Uppercase for hex), padded to a minimum number of digits (Pad
width, at most 1000) with zeros or spaces. A negative number keeps its sign before the zeros
(`-007`). Selections are taken in document order; an empty one inserts at the caret, a non-empty
one is replaced, and each leaves a caret after its text. With Pad short lines on, a caret at
the end of a line shorter than the rightmost caret that is not at a line end gets spaces first, so
the text lines up. A bad number or an overflow shows a message in the panel and changes nothing.
One undo step.

Pad to Column… inserts spaces at each caret until it reaches the rightmost caret's display
column; type a column number to pad to that column instead (1-based, up to 1000, in display
columns: the status bar's Column when the line has no tabs). Carets already past it and
selections are left alone; tabs before a caret are measured, never replaced.

The CSV/TSV commands work on the field holding the primary caret and on the whole document's
records, or only those a selection across several lines touches: Select Column (raw field text,
quotes included; empty fields become carets), Copy Column (values unquoted, one per line, an
empty line for a record without the column; recorded in the clipboard history), Move Column Left
/ Right (swaps the raw fields; records lacking either are unchanged), Delete Column (the field
and one delimiter) and Clear Column (empty, no quotes). Fields follow RFC 4180 leniently: a
field that starts with `"` is quoted and may hold the delimiter, `""` and line breaks (a multi-line
record), a quote elsewhere is literal, and empty fields are fields. An unclosed quote makes every
command say "Unclosed quote in the record starting on line N" and change nothing. Line endings, the
byte order mark and quoting are kept; each command is one undo step.

The delimiter is detected per document, from the first 200 records only: the one of tab, comma,
semicolon and pipe (in that order on a tie) whose records most often share one field count above
one, ignoring delimiters inside quotes. `.tsv` and `.tab` files mean tab, and `.csv` files mean
comma unless most records do not split on commas. A document with no such structure uses comma.
Edit > Columns > Delimiter overrides it for the buffer (all its views) until it closes; the menu
marks the choice with a `✓` in the item (gpui menu items have no check state).

## Text pipelines

A pipeline is a saved, ordered list of text steps, run on a document, its selections or a batch
of files. It lives in `~/Library/Application Support/Orlo/Pipelines/<name>.orlo-pipeline`, JSON
with `//` comments and trailing commas as the settings file has, and opens with JSON colours.
Each step is `{ "op": "…", options }`; an unknown operation or option, a bad regex or a bad table
is an error, so a typo is caught.

```json
{
	"name": "Tidy a list",
	"steps": [
		{ "op": "trim_trailing_white_space" },
		{ "op": "sort_lines_by_pattern", "pattern": "(\\d+)", "numeric": true },
		{ "op": "process_lines_matching", "pattern": "^#", "regex": true, "action": "delete" },
		{ "op": "apply_substitution_table", "rows": [["colour", "color"], ["grey", "gray"]] },
		{ "op": "add_line_prefix", "text": "- " },
	]
}
```

`sort_lines_by_pattern` also accepts `"order": "natural"`, `"locale"` (current macOS locale),
or `"length"` (extended grapheme clusters); `"lexical"` remains the default. Natural order
compares every ASCII integer chunk without overflow, so `v2-part9` precedes `v2-part10`;
equal numbers prefer fewer leading zeros after the remaining chunks compare. `"capture": 0`
uses the full regex match, a positive index selects that capture, and omission/`null` keeps
capture 1 when present or the full match. Invalid indices fail before editing. A valid optional
capture that did not participate is an empty key, as before. `"keep_header": true` keeps the
first line of each selected line block fixed (the first document line with no selection).
Reverse preserves equal-key source order; unmatched lines remain last in source order.
Disjoint selected line blocks sort independently and retain their mixed physical line endings
and final-newline state, with one Undo. The Numeric checkbox / `"numeric": true` retains finite
f64 sorting and lexical fallback for nonnumbers; combine it only with `"order": "lexical"`.
The same options are in Edit > Transform > Sort Lines by Pattern…; use `.*` to key whole lines.

The operations are the Edit > Transform ones (`sort_lines_by_pattern`, `process_duplicate_lines`,
`process_lines_matching` with `keep` or `delete` only, `add_line_prefix`, `add_line_suffix`,
`remove_line_prefix`, `remove_line_suffix`, `straighten_quotes`, `smart_quotes`,
`unicode_normalization`, `strip_diacritics`, `apply_substitution_table`) and `sort_lines`,
`reverse_lines`, `unique_lines`, `convert_case`, `trim_trailing_white_space`,
`remove_blank_lines`, `encode` and `decode` (Base64, Base64 URL-safe, hex, `uri_component`, `html_entities`, `json_string`), encode-only `regex_literal`, and `decode_text_escapes`. A substitution table is
given inline as `"rows": [["find", "replace"], …]` or as the path of a table file in
`"file"` (`~` works; a name with no folder is looked up beside the pipelines). Shuffle, and
copying or extracting matching lines, are not steps.

Tools > Text Pipelines: New Pipeline… asks for a name and opens a template listing every
operation and its options in comments (written by the first save); Edit Pipeline… opens a
saved one. Saving a `.orlo-pipeline` file checks it and shows every problem with its line in an
alert (the file is saved all the same). Preview Pipeline… shows the result for the document
(or selections) in a scratch tab, "Preview: name", and leaves the document alone. Run
Pipeline… applies every step in memory, then changes the document or each selection in one
undo step; a failing step (invalid Base64, say) stops the run with its number and message and
changes nothing.

Run Pipeline on Files… chooses files and folders (a folder asks which files, as a comma-separated
glob such as `*.txt, *.md`; it is walked honouring `.gitignore`) and runs in the background with
progress in the status bar. Cancel Pipeline Run stops it before the next file. The pipeline is
checked before the first file is touched. A file open in any Orlo window is changed in its buffer,
with the pipeline run on the buffer's text (its unsaved edits included), as one undo step, left
unsaved for review; the file on disk is not touched. Other files are read as UTF-8 or BOM-marked
UTF-16 (encoding and byte order mark are kept) and, when the pipeline changes it, replaced atomically keeping its permissions, line
endings and byte order mark. Files with malformed Unicode, a decoded NUL in the first 8 KB or that are
over 100 MB are skipped; a read-only file or a write that fails is reported as failed with the
original intact. A report opens in a scratch tab, "Pipeline Report: name", listing the changed,
unchanged, changed-in-open-buffer, skipped and failed files with each reason.

## Snippets

Snippets are `.sublime-snippet` files in `~/Library/Application Support/Orlo/Snippets`,
subfolders included. The folder is read again when its listing or a file changes, the next time a
snippet is looked up (at most once a second), so a new or edited file needs no restart.

Their format is the XML Sublime writes: `<content>` (usually CDATA), `<tabTrigger>`, `<scope>` (a scope
selector such as `source.rust - comment`; left out, the snippet applies everywhere) and
`<description>`. One layout newline at each end of `<content>` is dropped. Other elements are
skipped with what they hold, but a name one edit away from a known one (`<Scope>`, `<tabtrigger>`)
is an error that names the right one.

A trigger is matched against the text before the caret: `tab` expands the snippet
whose `<tabTrigger>` ends right there, if its scope matches where the trigger starts. A trigger that
starts with a word character needs a non-word character before it (or the start of the line), so
`fn` expands in `x fn` but not in `xfn`; one that starts with punctuation (`$$`, `\begin`) may
follow a word. The longest trigger wins. Several snippets with one trigger: the quick panel asks.
With several carets, all need the trigger and each expands on its own (its own `SELECTION`, line
and word). A snippet is also a row of the completion list (badge `s`), by its trigger, else its
description; while that list is open `tab` commits its selected row, which expands the snippet
when it is the one selected.

In the content, `$1` and `${1:default}` are tab stops (the default is selected), `$0` is where the
caret ends, a number used again is a mirror of that field, and a default may hold more fields
(`${1:a ${2:b}}`). `${1/regex/format/gim}` is a mirror run through a regex: the format has `$n` and
`${n}`, `\n`, `\t`, `\u`, `\l`, `\U`, `\L`, `\E` and `(?n:if:else)`, and the options are `g`, `i` and
`m`. `$NAME`, `${NAME}`, `${NAME:default}` and `${NAME/regex/format/options}` are variables:
`SELECTION`, `TM_SELECTED_TEXT`, `TM_CURRENT_LINE`, `TM_CURRENT_WORD`, `TM_FILENAME`,
`TM_FILEPATH`, `TM_DIRECTORY`, `TM_LINE_INDEX`, `TM_LINE_NUMBER`, `TM_SOFT_TABS`, `TM_TAB_SIZE` and
`TM_FULLNAME`; one that is empty or unknown gives its default, or nothing. `\$`, `\}` and `\\` are
literal, and so is a `$` that starts nothing. Lines after the first take the caret line's
indentation, and a tab becomes the view's indentation when it uses spaces.

Date/time variables use local time: `CURRENT_YEAR` (four digits), `CURRENT_MONTH`,
`CURRENT_DATE`, `CURRENT_HOUR`, `CURRENT_MINUTE`, `CURRENT_SECOND` (two digits), and
`CURRENT_SECONDS_UNIX` (decimal seconds since the Unix epoch). `CLIPBOARD` inserts plain text
literally, including Unicode and newlines. `WORKSPACE_NAME` uses the containing root's custom
label or folder name, `WORKSPACE_FOLDER` its path, and `RELATIVE_FILEPATH` the file path inside
it. Nested roots use the deepest containing root; untitled documents use the first root and
have no relative path. Missing clipboard/project values are empty; a file outside the roots
uses its filename for the relative path. All carets share one captured instant and clipboard/
project snapshot per expansion, including server snippets. Defaults and transforms work as
above; values never execute commands. Random and UUID variables are unsupported.

**Edit > Insert Date/Time…** (also in the Command Palette) inserts at every selection in one
Undo step. Its form defaults to local time and `%Y-%m-%dT%H:%M:%S%:z`, with a UTC choice.
Supported directives are `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%z` (`-0500`), `%:z` (`-05:00`)
and `%%` (literal `%`); other text is literal and unknown/incomplete directives reject the
whole insertion. The offset follows the zone at the captured instant, including daylight
saving changes. Custom keybindings/macros can use `editor::InsertDateTime` with optional
`format` and `utc` arguments (the same local/ISO defaults).

`${1|one,two,three|}` inserts the first choice and opens a list while that field is active.
Inside choices, `\,`, `\|` and `\\` escape a comma, pipe and backslash; values are literal text
and may be empty. `up` / `down` selects a value; `enter` or a click confirms it and leaves the
field selected for typing. `escape` dismisses the list and keeps the value; `tab` confirms and
visits the next field; `shift-tab` dismisses and visits the previous field. Returning to the
field opens its choices again. Confirmation updates every insertion caret and its mirrors and
transforms in one undo step. Typing, composition, a context or selection change, scrolling,
resizing or focus loss closes the list. Its values survive snippet-folder reloads and language
server shutdown.

In a snippet's fields `tab` selects the next field and `shift-tab` the previous
one; `tab` after the last numbered field goes to `$0` (or the end of the snippet's text) and ends
the session. `escape` ends it and keeps the selections. Typing in a field edits its mirrors with it.
A session also ends when the selection leaves its field, when an edit lands outside its fields
(one made in another view of the file too), when typing lands in a mirror alone (the text stays as
typed), and on undo, redo or a reload. With the completion list open the list wins: `tab` commits
its row, `escape` closes it, and `shift-tab` closes it and goes back a field.

Tools > Snippets > Insert Snippet… lists the snippets for the scope at the caret
(by description, else trigger, else file name) and expands the one chosen at the selections.
New Snippet… asks for a name, then opens that snippet, or a template that explains every part
in a new tab with the XML syntax; the file is written by its first save, as New Pipeline… does.
Both are in the Command Palette as "Snippets: …".

File > New From Template… (palette: File: New From Template…) searches all local snippets,
then asks for the new file's language. Confirming creates one dirty untitled file with normal
snippet variables and active fields; Save uses ordinary Save As. Cancel or a stale/invalid
definition creates nothing. The source editor is preserved, and templates execute no code.
See the [template guide](docs/user-guide/customization.md#snippets) for variable context and
runtime language fallback.

A snippet file that does not load (an XML error, no `<content>`, an unbalanced `${`, an
invalid regex or scope selector) is left out; one alert per load lists each such file and why, and
the other snippets work.

Limits: transform regexes have no lookaround, backreferences, `\g<name>` or `\Z`;
backtick shell interpolation is never run (the text stays literal);
sessions are per view and not saved.

## Animation

Showing or hiding the side bar slides it (150 ms), and Page Up/Down glide to their place (120 ms):
the caret moves at once, a second Page Down carries on from where the view is drawn, a wheel or
trackpad scroll takes over at once (its own momentum is never smoothed), and the glide stops at
the ends of the file. Clicks go where the text is drawn mid-glide, and a sliding side bar takes
clicks only where it is drawn. `"animation_enabled": false`, or macOS Reduce Motion (read again
whenever a window comes to the front), turns both off.

## Measuring

`ORLO_PERF=1` makes Orlo print its timings to stderr: `main()` to the first painted frame, each
key or scroll to the paint that shows it, and each frame's work. `ORLO_INDEX_TREE="$PWD/crates" cargo test
--workspace --locked --release -- --ignored --skip real_language_servers_smoke --nocapture --test-threads=1`
times loading, highlighting, typing, wrapping, searching, completion and index memory against
budgets (`benches/` and their owning regression tests); `ORLO_INDEX_TREE` is the folder to index (default
`target/lsp-tools/go`).

## Keys

| Keys | Action |
| --- | --- |
| `cmd-p` or `cmd-t` / `ctrl-g` | Goto Anything (`:N` previews a line, `@` switches to symbols, `escape` puts the caret back) / Goto Line |
| `cmd-r` / `f12` | Goto Symbol (previews as you browse) / Goto Definition (server first, project index fallback) |
| `shift-f12` | Goto Reference (lexical text matches with previews and `def` markers) |
| palette: Goto Definition to Side | Open in the next group; creates Columns 2 when needed |
| `cmd-shift-r` | Goto Symbol in Project (kind, name and path:line; Enter opens, Escape restores the view) |
| `cmd-alt-up` | Switch File: cycle same-folder counterparts |
| menu / palette: Show Outline; Related Files… | Toggle the symbol outline; open lexical related files |
| `ctrl--` / `ctrl-shift--` | Jump back / forward through goto targets |
| `cmd-shift-p` | Command Palette |
| `cmd-f` / `cmd-alt-f` | Find / Replace panel (`enter` next, `shift-enter` previous, `alt-enter` find all, `cmd-alt-enter` replace all, `ctrl-enter` a line break in the field, `alt-down` the field's history, `escape` close; `cmd-alt-r` / `cmd-alt-c` / `cmd-alt-w` / `cmd-alt-a` regex, case, whole word, Preserve Case; toggles for wrap, In Selection, highlight) |
| `cmd-shift-f` | Find in Files (`enter` finds, `cmd-alt-enter` previews Replace in Files, `tab` / `shift-tab` move between Find, Where and Replace, `alt-down` the field's history or the Where chooser, `escape` closes; toggles for regex, case, whole word, Show Context, Use Buffer, Use gitignore, Preserve Case) |
| (results browser) | `up` / `down`, `enter` open, `space` include or exclude, `left` / `right` collapse and expand, `tab` / `cmd-f` the filter, F4 / Shift-F4 the next / previous match, `escape` close |
| (menu and palette) | Find: Replace in Files…, Search Results, Refresh, Apply / Cancel Replacements, Find in Side Bar Selection… (`⌘`-click marks side bar rows), Regex Playground, Extract, Highlight, Saved Searches, Search History…, Clear Search History; Goto > Bookmarks: Bookmark Matching Lines, Invert, Copy / Cut / Delete / Keep Only / Paste to Bookmarked Lines |
| `cmd-g` / `cmd-shift-g` / `cmd-e` / `cmd-shift-e` | Find next / previous / use selection for find / for replace |
| `cmd-alt-e` | Replace the selected match and find the next |
| `cmd-d` / `ctrl-cmd-g` / `cmd-alt-g` | Quick add next (whole words only when it started from a caret) / quick find all / quick find (jump to the next occurrence) |
| `cmd-k cmd-d` | Quick skip next: drop the latest selection and take its next occurrence instead (wraps, passes selected ones) |
| `cmd-u` / `cmd-shift-u` | Undo / redo selection (Selection menu): the previous selections, exact ranges and primary, per view and up to 128 deep; a text edit in the view clears them (an edit in another view of the file moves them along), and unlike Sublime's soft undo they never undo text |
| `cmd-shift-l` | Split selection into lines |
| `cmd-l` / `cmd-alt-l` | Expand selection to line / to line upward |
| `cmd-shift-a` / `ctrl-shift-m` / `ctrl-m` | Expand selection (word, then brackets) / to brackets / jump to matching bracket |
| `ctrl-shift-up` / `ctrl-shift-down` | Add a caret on the line above / below |
| `cmd-click` | Add a cursor |
| `cmd-shift-click`, -double-, -triple-click and drag | Take a cursor, a word, a line or a range out of the selection |
| `alt-drag` | Box (column) selection |
| double-, triple-click and drag | Select by word, by line |
| drag from selected text | Move it (`drag_text`): the text lands where you let go, selected, as one undo step (`⌥` held at the release copies it, `escape` cancels; a click that does not move places a caret). Every selection is dragged, their texts joined by line breaks. Letting go in or against the dragged text, outside the text area (gutter, minimap) or over another view, group or window cancels |
| `escape` | Single selection |
| `cmd-shift-d` / `ctrl-shift-k` | Duplicate / delete line |
| `ctrl-cmd-up` / `ctrl-cmd-down` | Swap line up / down |
| `cmd-shift-j` / `ctrl-t` | Join lines / transpose |
| `cmd-enter` / `cmd-shift-enter` | Insert line after / before |
| `cmd-/` / `cmd-alt-/` | Toggle line / block comment |
| `cmd-]` / `cmd-[` / `tab` / `shift-tab` | Indent / unindent (`tab` fills to the next tab stop; a snippet's trigger and fields and an open completion list take `tab` first) |
| `alt-left/right`, `cmd-left/right`, `cmd-up/down` | Word, line (by row when wrapped), document motion (`shift` selects) |
| `ctrl-a` / `ctrl-e` | Start / end of the line, whatever the wrapping (`shift` selects) |
| `alt-backspace` / `alt-delete` | Delete word backward / forward |
| `ctrl-k` / `cmd-backspace` | Delete to end / beginning of line (`ctrl-y` yanks it back) |
| `cmd-k cmd-m`, `cmd-k cmd-a`, `cmd-k cmd-w`, `cmd-k cmd-x`, `cmd-k cmd-g` | Set mark, select / delete / swap to mark, clear mark |
| `cmd-alt-[` / `cmd-alt-]` | Fold / unfold (Edit > Code Folding): Fold hides exactly the text of a selection inside a line, the lines after the first of a multi-line selection, else the indented block the caret's line heads or, failing that, the innermost one around it; Unfold opens the folds on the lines of the carets and selections |
| `cmd-k cmd-1` … `cmd-k cmd-9` / `cmd-k cmd-0` | Fold Level 1–9 / Unfold All: level N folds every block whose header is indented N−1 levels (one level per tab size of indent), in place of the folds the view had; level 1, the menu's Fold All, folds the top-level blocks. Sublime's other key for Unfold All, `cmd-k cmd-j`, stays Orlo's Show / hide the panel |
| gutter ▾ / ▸ and a fold's `…` | Click ▾ to fold the block its line heads, ▸ to unfold it, or a fold's `…` marker to open that fold (folds inside it stay folded); the caret does not move (the ▾ shows while the pointer is over the gutter: `fold_buttons`, `fade_fold_buttons`) |
| (folded text) | A fold hides any range of text, per view, behind a three-column `…` marker; its exact ranges return with the session when the decoded text still matches, and are never part of the text, its undo history or a file save. See [fold persistence and limits](docs/user-guide/text-essentials.md#code-folding). A fold that hides line breaks joins its lines into one display line, the rest of its last line after the marker, and word wrap lays the marker out with the text. Up / Down, Page Up / Down, clicks, the minimap and the scroll bar go by the rows shown; Left / Right step over a fold as over one char, a caret at its start drawn before the marker and one at its end after it; `cmd-left` and `ctrl-a` after a fold go to its display line's start, never into the hidden text; a selection can span a fold, which stays folded, and Copy and Cut take the hidden text. Folds nest (an inner fold stays when its outer one opens); a fold over part of another takes it in; folds that only touch stay two. Find, Goto, bookmarks, Undo Selection and undo / redo that put a caret inside a fold open it, and so does an edit inside it; text typed at its start goes before it and text typed at its end after it, and other edits move it along |
| `cmd-k cmd-u` / `cmd-k cmd-l` | Upper / lower case (title, swap, camel, snake and kebab case in Edit > Convert Case) |
| (menu and palette) | Edit > Convert: Base64, Base64 URL-safe and Hex Encode / Decode of each selection (the whole document with none), URI Component Encode / Decode of selected text (no selection leaves text unchanged; UTF-8 bytes, uppercase %HH, only ASCII letters/digits and -._~ unescaped; Decode keeps literal + and whitespace; insertion retains the normal document line-ending policy, including CRLF-to-logical-LF normalization). Invalid input, including malformed escapes or non-UTF-8 output, changes no target and says which line. HTML Entity Escape / Unescape also requires selected text; Escape writes `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&#39;` for ampersand, angle brackets and quotes. Unescape decodes these and decimal/hex numeric Unicode scalar references once (`&amp;lt;` becomes `&lt;`), preserving unknown names, literal plus and whitespace. Numeric references require ASCII digits and a semicolon; malformed, overflow or surrogate values reject all targets. This specified subset does not perform HTML5 legacy control remapping. Styled Copy as HTML stays separate. Escape Regex, Quote / Unquote JSON String and Decode Text Escapes are also available for physical selections; see [literal escaping](docs/user-guide/text-essentials.md#literal-escaping) for strict Unicode dialects, failures and pipeline/keymap forms. All selections change in one Undo step. Copy MD5 / SHA-1 / SHA-256 / SHA-512 of the selections (one line each) or of the document as saved, and Hash File… (digests of files on disk, with Copy) |
| (menu and palette) | Edit > Document Statistics: an alert with the characters, grapheme clusters, words (UAX #29 segments holding a letter or digit), lines and encoded bytes as saved (encoding, line ending and BOM) of the document, and the selections' totals beside them when something is selected; large documents are counted off the UI thread. Edit > Character Info: each Unicode scalar of the cluster after the caret with its UTF-8 and UTF-16 hex and kind (no character names) |
| (menu and palette) | Edit > Columns: Column Editor… (text or numbers at every selection), Pad to Column…, Select / Copy / Move Left / Move Right / Delete / Clear Column for CSV and TSV, and Delimiter (see Columns and CSV/TSV) |
| (menu and palette) | Edit > Transform, on each selection (line commands on the lines it touches) or the whole document with none, one undo step, a bad pattern or table changes nothing and says why: Sort Lines by Pattern… (automatic or validated capture key; lexical, natural, current macOS locale or grapheme length; ignore case, numeric, reverse, keep first line as header; stable ties and unmatched lines last), Process Duplicate Lines… (by line or key; keep first / last, delete all duplicated, keep only duplicated; optionally copy the removed lines), Process Lines Matching… (regex or literal; keep, delete, copy to the clipboard or extract to a new document), Add / Remove Line Prefix / Suffix…, Straighten Quotes, Smart Quotes, Unicode Normalization (NFC, NFD, NFKC, NFKD), Strip Diacritics, and Apply Substitution Table… (a file of `find<TAB>replace` rows, `#` comments; literal in one pass with the longest find first, or regex rows in order with `$1`) |
| (menu and palette) | Tools > Text Pipelines: New / Edit Pipeline…, Preview Pipeline…, Run Pipeline…, Run Pipeline on Files…, Cancel Pipeline Run (see Text pipelines) |
| `ctrl-space` or `alt-escape` | The completion list (Edit > Show Completions), at the caret at once, even before a letter is typed (`alt-escape` is macOS's own completion key, for a Mac where `ctrl-space` is taken by input-source switching); with `auto_complete` on it also opens by itself as a word is typed (see Settings). On offer: the words near the caret in this buffer and those of the other open buffers of its syntax family, indexed symbols of that family, the syntax's keywords, the snippets whose scope fits, and, when either key or Goto > Language Server > Complete asked it, the language server's items (see Language intelligence); for a path in a string (`./src/ma`, `~/Doc`), the entries of its folder instead (relative to the file, `~/` or absolute). Typing narrows the list; a character that is no part of the word, a caret move, a selection, a scroll, a resize or leaving the window closes it. It does not open in a one-line field, while text is being composed or over a selection |
| (completion list open) | `tab` / `enter` apply the selected row and a click the row clicked: a word, keyword or path replaces the prefix at every caret that has it before it, as one undo step (a folder keeps the list open on its entries); a snippet expands at the carets that have the prefix and drops the others; a language server's item goes in at the primary caret only. `up` / `down` move; the wheel and the trackpad scroll the list and leave the selected row where it is; `escape` closes; `shift-tab` closes it and then does what it does without a list |
| `tab` after a snippet's trigger | Expands the snippet: the longest trigger that ends right before the caret, among the snippets whose scope fits there (several snippets with it: the quick panel asks); otherwise `tab` indents as usual (see Snippets) |
| (snippet session) | `tab` / `shift-tab` select the next / previous field, `escape` ends the session and keeps the selections (see Snippets) |
| (menu and palette) | Tools > Snippets: Insert Snippet…, New Snippet… (palette: Snippets: Insert Snippet…, Snippets: New Snippet…) |
| `f5` / `ctrl-f5` | Sort lines (reverse, unique and shuffle in Edit > Permute Lines) |
| `f6`, `ctrl-f6` / `ctrl-shift-f6` | View > Spell Check for this view; Next / Previous Misspelling across the document, wrapping once (Spelling Suggestions… in View, the palette or right-click) |
| (menu and palette) | Tools > Compare > Compare Files… / Compare Folders…; Previous / Next Difference, Refresh and per-hunk Apply in the Comparison panel; confirmed Copy File to Left/Right for a supported one-sided file |
| `cmd-alt-q` / `cmd-shift-v` | Wrap paragraph at 78 columns / paste and indent |
| `(` `[` `{` `"` `'` | Auto-pair, wrap a selection, step over the closer; `enter` between `{}` opens a block (`'` stays single in Rust, for lifetimes) |
| `cmd-c` / `cmd-x` / `cmd-v` | With no selection, copy or cut whole lines; those paste back above the caret's line, one per caret when the counts match |
| `cmd-k cmd-v` | Paste from History: pick one of the last 15 texts copied by Copy, Cut, Copy Column or the copy options of Process Duplicate Lines and Process Lines Matching, pasted like `cmd-v` (Edit > Clear Clipboard History empties the list, not the system clipboard); kept in memory only |
| (menu and palette) | Edit > Copy as HTML (no key): the selections, joined by line breaks as Copy does, or the whole document with none selected, as HTML in the editor's font and colour scheme (colour, bold, italic, underline and background per run; lines over 20,000 bytes stay plain) together with the plain text in one pasteboard write, so a rich-text editor pastes colours and a plain one text; the text goes into the clipboard history. Built off the UI thread (the status bar says "Working…" above 4M characters); more than 16 MB of text is refused |
| `cmd-f2`, `f2`, `shift-f2`, `cmd-shift-f2`, `alt-f2` | Toggle / next / previous / clear / select all bookmarks |
| `ctrl-l` / `ctrl-alt-up` / `ctrl-alt-down` | Scroll to selection / scroll one line up / down |
| `cmd-z` / `cmd-shift-z` (or `cmd-y`) | Undo / redo |
| `cmd-n`, `cmd-o`, `cmd-s`, `cmd-shift-s`, `cmd-alt-s`, `cmd-w`, `cmd-shift-t` | New, open, save, save as, save all, close tab, reopen closed file |
| `cmd-shift-n` / `cmd-shift-w` / `ctrl-cmd-f` | New window / close window / full screen |
| `cmd-1`…`cmd-8`, `cmd-9`, `cmd-shift-]` / `cmd-shift-[`, `ctrl-tab` | Switch tabs in the focused group (`cmd-9` is the last tab) |
| `cmd-,` | Settings |
| `cmd-=` / `cmd--` | Larger / smaller font in this window (reset in Orlo > Settings… > Font) |
| `cmd-k cmd-b` | Toggle side bar |
| (menu and palette) | View > Show White Space (palette: View: Toggle White Space): marks all white space in this window, as `draw_white_space` `"all"` does (✓ while on); the item again goes back to the setting, or to `"none"` when the setting is `"all"`; the choice is kept with the window. Marks: `·` space, `→` tab with a line to the next tab stop, `°` other Unicode spaces (U+00A0, U+2000–U+200A, U+202F, U+205F, U+3000), `¬` line end (`"all"` only); in the indent guide colour, and inside a selection halfway to the text colour (inside the find panel's current match they keep the plain indent guide colour); a folded header draws its own marks; the lines it hides draw none |
| `cmd-alt-1` … `cmd-alt-5` / `cmd-alt-shift-2`, `cmd-alt-shift-3` | Layout: Single, Columns 2, 3, 4, Grid 4 / Rows 2, 3 |
| `ctrl-1` … `ctrl-9` / `ctrl-shift-1` … `ctrl-shift-9` | Focus group N / move the file to group N |
| `cmd-k cmd-right` / `cmd-k cmd-left` | Focus the next / previous group |
| `cmd-k cmd-shift-right` / `cmd-k cmd-shift-left` | Move the file to the next / previous group |
| (menu and palette) | View > Groups: Split Right, Split Down, Move File to New Group Right / Down, Close Group; View > Layout: Save Layout…, Apply / Delete Saved Layout…; View > Sync Scrolling: Vertical, Horizontal |
| `⌘`-click / `⇧`-click a tab, `cmd-alt-shift-right` / `cmd-alt-shift-left`, `cmd-k cmd-]` / `cmd-k cmd-[` | Show tabs side by side; add the next / previous tab; move the input among them |
| `cmd-k enter` / `cmd-k cmd-t` | Keep a preview tab open / the tab list (`▾` on a strip: its group's; `cmd-backspace` in the list closes a tab) |
| (menu and palette) | View > Tabs: Pin / Unpin Tab, Close Other Tabs, Close Tabs to the Right, Deselect Other Tabs, Move to New Window, All Open Tabs… |
| `ctrl-cmd-p` | Switch Project (Project: Open Project…, Save Project As…, Close Project, Edit Project, Save Workspace As…, Remove Folder…, Move Folder Up / Down) |
| `ctrl-0`, then `up` `down` `left` `right` `enter` `space` `f2` `cmd-backspace` `escape` | The side bar: move, collapse / expand, open, preview, rename, move to the Trash, back to the editor (right click a row for New File…, New Folder…, Move…, Duplicate…, Reveal in Finder, Find in Folder…) |
| `cmd-k cmd-j` | Show / hide the panel (View > Panel: Collapse, Focus Panel, Move Panel to Bottom / Right) |
| `ctrl-cmd-shift-f` | Enter / exit distraction-free mode |

The supported Unicode and legacy encoding set, plus explicit reopen and save-conversion
workflows, are listed in [File encodings](docs/user-guide/text-essentials.md#file-encodings).

## Not in this version

Language limits: Orlo does not download or build grammars; a language folder brings its own
`.wasm`, built with the tree-sitter CLI. There is no debugger and no plugin runtime: the
WebAssembly sandbox runs Tree-sitter grammars only, and packages are shell commands. Typing
indents only on Enter: by a folder's `indents.scm` where it has one, else by Orlo's existing
rules. A language with neither a Sublime grammar nor a `tags.scm` (Nim) has an empty Goto Symbol.

Sublime's View > Groups > Max Columns and an online package marketplace
(Incremental Find opens the Find panel), Find Results colouring, FSEvents,
tag commands, Sublime's soft undo (`cmd-u` there also steps back through text edits;
Orlo's Undo Selection never touches text), Windows/Linux.

Workspace limits: a project is open in one window at a time. Switching projects keeps unsaved
text in the workspace left behind; an untitled file that also has a view in another window comes
back from the workspace as a copy. Rewriting a project file loses its comments. Moving across
volumes is refused. A tab dragged between windows lands at the end of the group under the
pointer (within a window the insertion line says exactly where); the strip scrolls at its ends
only while the pointer moves. Overlapping folders share their expanded state for the folders they
share. The side bar's `.gitignore` dimming still reads plain names from each folder's own
`.gitignore` only. Synchronized scrolling follows lines, not wrapped rows. Saving the settings
file while in distraction-free mode resets the window's View menu choices made in the mode.

Search limits: Preserve Case looks at the whole match (VS Code also matches the case of each part
of `foo-bar` or `foo_bar`; Orlo does not). The lookaround engine tries each start position on its
own (about 25 ns a character), so a lookaround search of a 100 MB file takes a few seconds on
another thread; Find in Files reads each file (up to 100 MB) whole into memory. Extract from search results
takes every match, whatever the browser's checkboxes or filter. With Use Buffer off, a clean open
file that Replace in Files writes reloads when Orlo next checks it (at once in the front window,
when another window comes to the front). `<open files>` is this window's tabs. A Find Results
header shows a query's line breaks as `\n`. Bookmarks and highlights belong to each view;
highlights and the search histories are not saved with the session.

Known limits: the mark and Reopen Closed File live in memory only. Swapping a line past a
bookmarked line carries that bookmark onto the swapped line. A file with mixed line endings saves with the majority one,
as in Sublime: CRLFs in an LF file are saved as LF and a lone LF in a CRLF file as CRLF, while
stray lone CRs are kept as content. Changing line
endings is not an undo step, as in Sublime. Always Ignore lasts until the tab closes. A window saved in full
screen comes back one title bar taller; restoring onto a second display has not been tried on two
monitors. Word wrap counts columns (a CJK char or emoji as two), so a proportional font can run past
the edge. Wrap Paragraph keeps comment and quote prefixes but gives list items no hanging
indent. Resizing a wrapped view re-wraps the lines on screen at once and the rest of
the file over the next frames, so the scroll bar can settle a moment after the drag. Lines over
20,000 bytes (minified files) are drawn in the colour of the scope they start in;
Sublime colours them, slowly. Such a line is not parsed at all, so a string or comment it opens or
closes is missed and the lines after it are coloured as if it were not there.
Find Results grows without limit, as in Sublime. A frame
colours at most 50 lines (16 KB) past what is already coloured; lines further away (after a far jump or a fast fling) are
drawn plain for a frame or two while another thread colours them (lines below an edit keep their
old colours meanwhile), carrying on from the coloured part for up to 20,000 lines. Past that it restarts 200 lines above the target, so a block
comment opened earlier is miscoloured until you scroll up to it. In a buffer over 4M
characters, find as you type searches on another thread, and so do the CSV/TSV column commands
(the status bar says "Working…", and "The document changed; run the command again" if the text or
the primary caret changed meanwhile). A smaller document is parsed at once on the UI thread. The
commands read every record (only delimiter detection is capped, at 200 records), and display
columns treat only the main East Asian wide and emoji blocks as two columns wide.
Sideways scrolling is limited to the widest line on screen. Unwrapped lines over 4,096
characters are shaped only around the view, counting each character as one column, so tabs and
wide characters before the view shift where such a line's text is drawn and clicked. Files over
4 MB are read in the background, at launch too: their tab appears when the read is done. `ctrl-1` … `ctrl-9` reach Orlo only when macOS
Mission Control's "Switch to Desktop N" shortcuts are off (Sublime has the same clash).
The side bar
follows linked folders when you expand them. Toggle Comment in a language with only block
comments finds the comment around the caret by its text, not by syntax scope as Sublime does, so
a `/*` inside a string counts. Shift-double-click and shift-triple-click leave a caret where you
clicked, as Sublime 4215 does, rather than extending by word or line. With `drag_text` on (the
default) an unmodified press inside a selection starts a text drag; a press with a modifier (`cmd`,
`shift`, `alt`, `cmd-shift`) keeps its meaning, so `cmd-shift-click` inside a selection still splits
it, as Sublime does with `drag_text` off (with it on, Sublime's press there collapses the
selections); where Sublime would leave no selection at all, Orlo leaves a caret at the click. A text
drag stays within its view: released in another view, group or window, outside the text, or in or
against the dragged text, it is cancelled and nothing changes. The exception is a second Orlo
window that overlaps the first: gpui cannot tell the windows apart there, so the text drops into
the window where the drag started (one undo step takes it back). Folds use syntax boundaries
when available, with indentation fallback and exact inline selections; valid per-view ranges
return in sessions and workspaces within the [persistence limits](docs/user-guide/text-essentials.md#code-folding).
A save whose `_on_save` cleanup changes hidden lines opens their folds.
Copy as HTML carries the syntax colours, not a language server's semantic colours. A big Copy as
HTML keeps the highlighting thread until its page is built, so colouring elsewhere waits for it.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md). Report vulnerabilities privately, as
[SECURITY.md](SECURITY.md) describes.

## Licence

MIT, see [LICENSE](LICENSE).

The bundled languages in `assets/languages/` carry third-party grammars, queries and syntax
material. Each folder's `LICENSE` (`assets/languages/*/LICENSE`) says what came from which
repository and revision, followed by that licence. The Nim grammar and its highlight query are
MPL-2.0; their source is the repository and revision in `assets/languages/nim/LICENSE`.
`vendor/vt100` is a vendored copy of the vt100 crate 0.16.2 (MIT, `vendor/vt100/LICENSE`), and
the chrome icons follow Lucide's drawings (ISC, `assets/icons/LICENSE`). Orlo's other source
vendors no third-party code. Sublime Text, Pulsar, Nextpad++, TextMate and BBEdit
informed its features, not its code. A bounded check on
2026-10-17 compared Orlo's source with GPUI 0.2.2's `src/` and `examples/` and the cited
Nextpad++/Pulsar files, and scanned the commit history; it found no copied implementation.
That is not legal clearance. The key-window guard in `crates/ui/src/platform/macos/mod.rs` is Orlo's own code,
written from a diagnosis of [zed#50151](https://github.com/zed-industries/zed/issues/50151).
If copied code is found,
record the upstream revision and the Orlo ranges, keep its notices, and rewrite it
independently with regression tests.
