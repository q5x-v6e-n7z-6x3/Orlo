use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    Ours,
    Theirs,
    Both,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictBlock {
    pub range: Range<usize>,
    pub ours: Range<usize>,
    pub theirs: Range<usize>,
}

fn marker(line: &str) -> Option<(u8, usize)> {
    let first = *line.as_bytes().first()?;
    if !matches!(first, b'<' | b'|' | b'=' | b'>') {
        return None;
    }
    let width = line.bytes().take_while(|b| *b == first).count();
    (width >= 7).then_some((first, width))
}

pub fn has_conflict_markers(text: &str) -> bool {
    text.lines().any(|line| marker(line).is_some())
}

/// Byte ranges include the marker lines, while choices contain only their original text.
pub fn conflicts(text: &str) -> Vec<ConflictBlock> {
    struct Pending {
        start: usize,
        ours_start: usize,
        ours_end: Option<usize>,
        theirs_start: Option<usize>,
        width: usize,
        depth: usize,
        invalid: bool,
    }
    let mut result = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let end = offset + line.len();
        let marked = marker(line);
        let valid = marked.is_none_or(|(kind, width)| {
            let suffix = line[width..].trim_end_matches(['\r', '\n']);
            suffix.is_empty() || (kind != b'=' && suffix.starts_with(' '))
        });
        if let (false, Some(block)) = (valid, &mut pending) {
            block.invalid = true;
        }
        match marked {
            Some((b'<', width)) => match &mut pending {
                Some(block) => {
                    block.depth += 1;
                    block.invalid = true;
                }
                None => {
                    pending = Some(Pending {
                        start: offset,
                        ours_start: end,
                        ours_end: None,
                        theirs_start: None,
                        width,
                        depth: 1,
                        invalid: !valid,
                    })
                }
            },
            Some((b'>', width)) => {
                if let Some(block) = &mut pending {
                    if block.depth > 1 {
                        block.depth -= 1;
                    } else {
                        let block = pending.take().unwrap();
                        if !block.invalid && width == block.width {
                            if let (Some(ours_end), Some(theirs_start)) = (block.ours_end, block.theirs_start) {
                                result.push(ConflictBlock {
                                    range: block.start..end,
                                    ours: block.ours_start..ours_end,
                                    theirs: theirs_start..offset,
                                });
                            }
                        }
                    }
                }
            }
            Some((kind, width)) => {
                if let Some(block) = &mut pending {
                    if block.depth == 1 {
                        if width != block.width
                            || block.theirs_start.is_some()
                            || (kind == b'|' && block.ours_end.is_some())
                        {
                            block.invalid = true;
                        } else if kind == b'|' {
                            block.ours_end = Some(offset);
                        } else {
                            block.ours_end.get_or_insert(offset);
                            block.theirs_start = Some(end);
                        }
                    }
                }
            }
            None => {}
        }
        offset = end;
    }
    result
}

pub fn resolve_conflict(text: &str, block: usize, choice: ConflictChoice) -> Result<String, String> {
    let blocks = conflicts(text);
    let block = blocks.get(block).ok_or_else(|| "Choose a complete conflict block".to_string())?;
    let mut result = text[..block.range.start].to_owned();
    if matches!(choice, ConflictChoice::Ours | ConflictChoice::Both) {
        result.push_str(&text[block.ours.clone()]);
    }
    if matches!(choice, ConflictChoice::Theirs | ConflictChoice::Both) {
        result.push_str(&text[block.theirs.clone()]);
    }
    result.push_str(&text[block.range.end..]);
    Ok(result)
}
