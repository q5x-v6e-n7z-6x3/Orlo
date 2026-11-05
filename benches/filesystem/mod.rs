use super::*;

/// The foreground part of the 500 ms disk poll with many documents and cloned views.
#[gpui::test]
#[ignore]
fn bench_disk_poll_many_documents(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    let (ws, cx) = crate::workspace::ui_tests::window(cx);
    ws.update_in(cx, |ws, window, cx| {
        for i in 0..1_024 {
            let path = PathBuf::from(format!("/orlo-poll-benchmark/{i}.txt"));
            let editor = ws.new_editor(EditorState::new(), cx);
            editor.update(cx, |e, cx| e.update_doc(cx, |d| d.path = Some(path.clone())));
            let clone = editor.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
            ws.views.add(editor);
            ws.views.add(clone);
            // Model a stat already in flight; measure UI work, with no disk/subprocess noise.
            ws.checking.insert(path);
        }
        ws.poll_disk(window, cx);
        let mut samples = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            for _ in 0..10 {
                ws.poll_disk(window, cx);
            }
            samples.push(start.elapsed().as_secs_f64() * 100.);
        }
        samples.sort_by(f64::total_cmp);
        println!("bench disk poll, 1024 documents/2048 views: {samples:?} ms/poll");
        assert!(samples[3] < 1., "disk poll UI work exceeded 1 ms");
        assert_eq!(ws.checking.len(), 1_024);
    });
}

#[test]
#[ignore]
fn bench_load_100mb() {
    let p = big_file();
    time("load 100 MB (read, decode, rope)", 3, 1500., || EditorState::load(&p).unwrap());
}

/// Saving a 100 MB document: the UI thread's part, the cleanup the default settings ask for, the
/// capture and the finish, is a snapshot and some bookkeeping. Encoding, file history and the
/// write are another thread's: the UI thread copies no text and writes nothing.
#[test]
#[ignore]
fn bench_save_100mb_on_the_ui_thread() {
    use orlo_editor::text::ByteOffset;
    let p = big_file();
    let mut s = EditorState::load(&p).unwrap();
    s.insert("x");
    let to = std::env::temp_dir().join("orlo-bench-save-100mb.rs");
    let _ = std::fs::remove_file(&to);
    let cleanup = orlo_config::settings::Settings::default().cleanup();
    let mut capture = || {
        s.cleanup_for_save(&cleanup);
        s.doc.capture_save(to.clone(), None).unwrap()
    };
    time("save 100 MB: the UI thread's capture", 5, 1., &mut capture);
    let (req, bytes) = allocated_at_most(&mut capture);
    assert!(bytes < 1 << 20, "the capture allocated {bytes} bytes: it copies the text");
    assert!(!to.exists(), "the capture wrote the file");
    // the other thread's part writes it all; the finish only records it
    let written = req.write().unwrap();
    let size = std::fs::metadata(&p).unwrap().len() as usize;
    assert_eq!(written.bytes, ByteOffset(size + 1));
    time("save 100 MB: the UI thread's finish", 5, 1., || s.doc.finish_save(&req, &written));
    assert!(!s.doc.dirty);
    std::fs::remove_file(&to).unwrap();
}

#[test]
#[ignore]
fn bench_load_10mb_line() {
    let p = temp_file("orlo-bench-10mb-line.json", || format!("[{}]\n", "{\"k\": 12345},".repeat(800_000)));
    time("load a 10 MB line", 3, 300., || EditorState::load(&p).unwrap());
}

#[test]
#[ignore]
fn bench_find_in_files_one_99mb_line() {
    let p = temp_file("orlo-bench-line99mb.txt", || {
        let mut s = "abc def ".repeat(99 << 17);
        s.push_str("needle");
        s
    });
    let o = FindOpts::default();
    time("find in files, one 99 MB line", 3, 1500., || {
        orlo_editor::searching::search::search_file(&p, None, "needle", &o)
    });
}

/// The project index's memory on a real tree, the Go toolchain's source (which the Go language
/// server's setup puts in target/lsp-tools), or `ORLO_INDEX_TREE`: what dropping the index frees,
/// by the counting allocator, and what its entries hold by part, each a file and at 100,000 files.
#[gpui::test]
#[ignore]
fn bench_index_memory(cx: &mut gpui::TestAppContext) {
    use crate::languages::index::{Entry, EntryKey, IndexedSymbol, ProjectIndex};
    use gpui::AppContext as _;
    use std::mem::size_of;
    use std::sync::atomic::Ordering::Relaxed;
    let tree = (std::env::var_os("ORLO_INDEX_TREE").map(PathBuf::from))
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/lsp-tools/go"));
    assert!(tree.is_dir(), "no Go tree at {}", tree.display());
    cx.update(|cx| cx.set_global(orlo_config::settings::Settings::default()));
    LIVE.store(0, Relaxed);
    COUNT.store(true, Relaxed);
    let index = cx.new(ProjectIndex::new);
    let start = Instant::now();
    let name = tree.display().to_string();
    let root = orlo_project::project::Root::new(tree);
    index.update(cx, |i, cx| i.set_roots(vec![root], None, cx));
    while index.read_with(cx, |i, _| i.progress().is_some()) {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(1));
        assert!(start.elapsed() < Duration::from_secs(1800), "still building");
    }
    let took = start.elapsed();
    let built = LIVE.load(Relaxed);
    // strings and vectors at their capacity; a hash map's buckets (capacity / 7 x 8), each a
    // slot and a control byte; the structs themselves. Parts: symbols (their structs, names,
    // kinds and signatures), docs, words (each entry's counts table), shared words (each
    // distinct word once with its reference counts, and the word table's buckets), the rest
    // (the entry, its paths and syntax name).
    let buckets = |capacity: usize| capacity / 7 * 8;
    let parts = |e: &Entry| {
        let key = match &e.key {
            EntryKey::File(p, _) => p.capacity(),
            EntryKey::Buffer(..) => 0,
        };
        let path = e.path.as_ref().map_or(0, |p| p.capacity());
        let texts: usize =
            (e.symbols.iter()).map(|s| s.name.capacity() + s.kind.capacity() + s.signature.capacity()).sum();
        let docs: usize = (e.symbols.iter()).map(|s| s.doc.as_ref().map_or(0, String::capacity)).sum();
        let table = buckets(e.idents.capacity()) * (size_of::<(std::sync::Arc<str>, u32)>() + 1)
            + buckets(e.members.capacity()) * (size_of::<(String, u32)>() + 1)
            + e.members.keys().map(String::capacity).sum::<usize>()
            + 4 * size_of::<usize>()
            + 2 * size_of::<std::collections::HashMap<(), ()>>();
        [
            e.symbols.capacity() * size_of::<IndexedSymbol>() + texts,
            docs,
            table,
            0,
            size_of::<Entry>() + key + path + e.syntax.capacity(),
        ]
    };
    let (files, symbols, words, held) = index.read_with(cx, |i, _| {
        let entries: Vec<&Entry> = i.entries().collect();
        let mut held = [0; 5];
        let mut shared = std::collections::HashSet::new();
        for e in &entries {
            for (h, b) in held.iter_mut().zip(parts(e)) {
                *h += b;
            }
            for w in e.idents.keys() {
                if shared.insert(w.as_ptr()) {
                    held[3] += 2 * size_of::<usize>() + w.len();
                }
            }
        }
        held[3] += shared.len() * 8 / 7 * (size_of::<std::sync::Arc<str>>() + 1);
        (
            entries.len(),
            entries.iter().map(|e| e.symbols.len()).sum::<usize>(),
            entries.iter().map(|e| e.idents.len()).sum::<usize>(),
            held,
        )
    });
    drop(index);
    // a dropped entity is released when an update ends
    cx.update(|_| {});
    cx.run_until_parked();
    let freed = (built - LIVE.load(Relaxed)).max(0) as usize;
    COUNT.store(false, Relaxed);
    let mb = |b: usize| b as f64 / f64::from(1 << 20);
    println!("bench index of {name}: {files} files, {symbols} symbols, {words} words, built in {took:.1?}");
    let rows = [
        ("left allocated by it", built.max(0) as usize),
        ("freed by dropping it", freed),
        ("entries' contents", held.iter().sum()),
        ("  symbols", held[0]),
        ("  docs", held[1]),
        ("  words", held[2]),
        ("  shared words", held[3]),
        ("  the rest", held[4]),
    ];
    for (what, b) in rows {
        let each = b / files.max(1);
        println!(
            "  {what:<22} {:>8.1} MB  {each:>7} bytes a file  {:>8.1} MB at 100,000 files",
            mb(b),
            mb(each * 100_000)
        );
    }
}
