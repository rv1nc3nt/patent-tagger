//! Structure of a document's stored full text, for the View screen's
//! outline and navigation: the description split into headings and
//! paragraphs, the claims split into claims with their references to other
//! claims. Pure functions over the plain text stored by `fulltext`.
//!
//! Offsets are UTF-16 code units into the section's text, the unit the
//! webview's strings and selections use, so the UI slices the text with
//! them directly. See also `annotations`, which uses the same unit.

/// Longest line still taken for a heading when it has no paragraph number.
const MAX_HEADING_LEN: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Heading,
    Paragraph,
    Claim,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Block {
    pub kind: BlockKind,
    /// Paragraph number without brackets (`"0001"`) or claim number
    /// (`"1"`); `None` for headings and unnumbered paragraphs.
    pub label: Option<String>,
    pub start: usize,
    pub end: usize,
    /// Claims only: the earlier claims this one refers to, ascending.
    pub depends_on: Vec<u32>,
    /// Claims only: the spans of text that name another claim, for links.
    pub claim_refs: Vec<ClaimRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ClaimRef {
    pub start: usize,
    pub end: usize,
    pub claim: u32,
}

/// One line with its UTF-16 span in the whole text.
struct Line<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

fn lines(text: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    for line in text.split('\n') {
        let len = utf16_len(line);
        out.push(Line {
            text: line,
            start: pos,
            end: pos + len,
        });
        pos += len + 1;
    }
    out
}

pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// The description, one block per non-empty line. `fulltext` stores one
/// paragraph or heading per line (SPEC 5.5). A line starting with `[0001]`
/// is a numbered paragraph. An unnumbered line is a heading when it is
/// short, starts with a capital letter and does not end like a sentence,
/// and a paragraph otherwise.
pub fn description_blocks(text: &str) -> Vec<Block> {
    lines(text)
        .into_iter()
        .filter(|l| !l.text.trim().is_empty())
        .map(|l| {
            let (kind, label) = match paragraph_number(l.text) {
                Some(n) => (BlockKind::Paragraph, Some(n.to_string())),
                None if looks_like_heading(l.text) => (BlockKind::Heading, None),
                None => (BlockKind::Paragraph, None),
            };
            Block {
                kind,
                label,
                start: l.start,
                end: l.end,
                depends_on: Vec::new(),
                claim_refs: Vec::new(),
            }
        })
        .collect()
}

fn paragraph_number(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('[')?;
    let close = rest.find(']')?;
    let number = &rest[..close];
    (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then_some(number)
}

fn looks_like_heading(line: &str) -> bool {
    let line = line.trim();
    line.chars().count() <= MAX_HEADING_LEN
        && !line.ends_with(['.', ',', ';'])
        && line
            .chars()
            .find(|c| c.is_alphabetic())
            .is_some_and(char::is_uppercase)
}

/// The claims, one block per claim. A line starting with a number followed
/// by `.` or `)` opens a claim; any other line continues the previous one
/// (text before the first numbered line forms an unnumbered block).
pub fn claim_blocks(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut claim_numbers: Vec<Option<u32>> = Vec::new();
    for line in lines(text) {
        if line.text.trim().is_empty() {
            continue;
        }
        let number = claim_number(line.text);
        match (number, blocks.last_mut()) {
            (None, Some(last)) => last.end = line.end,
            _ => {
                blocks.push(Block {
                    kind: BlockKind::Claim,
                    label: number.map(|n| n.to_string()),
                    start: line.start,
                    end: line.end,
                    depends_on: Vec::new(),
                    claim_refs: Vec::new(),
                });
                claim_numbers.push(number);
            }
        }
        if let Some(block) = blocks.last_mut() {
            let own = claim_numbers.last().copied().flatten();
            add_references(block, own, line.text, line.start);
        }
    }
    for block in &mut blocks {
        block.depends_on.sort_unstable();
        block.depends_on.dedup();
    }
    blocks
}

fn claim_number(line: &str) -> Option<u32> {
    let line = line.trim_start();
    let digits = line.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || !matches!(line.as_bytes().get(digits), Some(b'.') | Some(b')')) {
        return None;
    }
    line[..digits].parse().ok()
}

/// Finds the references to other claims in one line of a claim: `claim 1`,
/// `claims 1 or 2`, `claims 1 to 3`, `claims 1-3`, `any one of claims 1,
/// 2 and 5`, and `any of the preceding claims` (every earlier claim, with
/// no link span).
fn add_references(block: &mut Block, own: Option<u32>, line: &str, line_start: usize) {
    let lower = line.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut search_from = 0;
    while let Some(found) = lower[search_from..].find("claim") {
        let at = search_from + found;
        search_from = at + "claim".len();
        if at > 0 && bytes[at - 1].is_ascii_alphabetic() {
            continue;
        }
        let mut pos = at + "claim".len();
        if bytes.get(pos) == Some(&b's') {
            pos += 1;
        }
        if bytes.get(pos).is_some_and(|b| b.is_ascii_alphabetic()) {
            continue; // "claimed", "claiming"
        }

        if is_all_earlier_claims(&lower[..at]) {
            if let Some(own) = own {
                block.depends_on.extend(1..own);
            }
            continue;
        }

        let mut first = true;
        let mut range_from: Option<u32> = None;
        loop {
            let after_space = skip_spaces(bytes, pos);
            let digits = bytes[after_space..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            if digits == 0 {
                break;
            }
            let Ok(number) = lower[after_space..after_space + digits].parse::<u32>() else {
                break;
            };
            let span_start = if first { at } else { after_space };
            block.claim_refs.push(ClaimRef {
                start: line_start + utf16_len(&line[..span_start]),
                end: line_start + utf16_len(&line[..after_space + digits]),
                claim: number,
            });
            match range_from.take() {
                Some(from) if from < number => block.depends_on.extend(from..=number),
                _ => block.depends_on.push(number),
            }
            first = false;
            pos = after_space + digits;

            // A separator continues the list; anything else ends it.
            let next = skip_spaces(bytes, pos);
            let rest = &lower[next..];
            if let Some(len) = ["to", "-", "\u{2013}"]
                .iter()
                .find(|s| rest.starts_with(*s))
                .map(|s| s.len())
            {
                range_from = Some(number);
                pos = next + len;
            } else if let Some(len) = [",", "or", "and"]
                .iter()
                .find(|s| rest.starts_with(*s))
                .map(|s| s.len())
            {
                pos = next + len;
            } else {
                break;
            }
        }
        if let Some(own) = own {
            block.depends_on.retain(|&n| n < own);
        }
    }
}

fn skip_spaces(bytes: &[u8], mut pos: usize) -> usize {
    while bytes.get(pos).is_some_and(|b| b.is_ascii_whitespace()) {
        pos += 1;
    }
    pos
}

/// Whether the words just before "claims" say "the preceding", "the
/// foregoing" or "the previous".
fn is_all_earlier_claims(before: &str) -> bool {
    let before = before.trim_end();
    ["preceding", "foregoing", "previous", "above"]
        .iter()
        .any(|w| before.ends_with(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(text: &str, start: usize, end: usize) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        String::from_utf16(&units[start..end]).unwrap()
    }

    #[test]
    fn description_numbered_paragraphs_and_headings() {
        let text = "FIELD OF THE INVENTION\n[0001] The invention relates to bricks.\n\nBackground\n[0002] Known apparatus.";
        let blocks = description_blocks(text);
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].kind, BlockKind::Heading);
        assert_eq!(
            slice(text, blocks[0].start, blocks[0].end),
            "FIELD OF THE INVENTION"
        );
        assert_eq!(blocks[1].kind, BlockKind::Paragraph);
        assert_eq!(blocks[1].label.as_deref(), Some("0001"));
        assert_eq!(blocks[2].kind, BlockKind::Heading);
        assert_eq!(
            slice(text, blocks[3].start, blocks[3].end),
            "[0002] Known apparatus."
        );
    }

    #[test]
    fn unnumbered_sentences_are_paragraphs_not_headings() {
        let blocks = description_blocks(
            "A short sentence.\nWhat is claimed is:\nfigure 5 is a perspective view of a part; and",
        );
        assert_eq!(blocks[0].kind, BlockKind::Paragraph);
        assert_eq!(blocks[0].label, None);
        assert_eq!(blocks[1].kind, BlockKind::Heading);
        // A list item of the figure descriptions (EP1000000).
        assert_eq!(blocks[2].kind, BlockKind::Paragraph);
    }

    #[test]
    fn offsets_are_utf16_units() {
        // "𝛼" is two UTF-16 units and four UTF-8 bytes.
        let text = "[0001] 𝛼 é\n[0002] next";
        let blocks = description_blocks(text);
        assert_eq!(slice(text, blocks[1].start, blocks[1].end), "[0002] next");
        assert_eq!(blocks[0].end, 11);
    }

    #[test]
    fn claims_with_dependencies() {
        let text = "1. Apparatus for bricks.\n\
                    2. Apparatus as claimed in claim 1, wherein X.\n\
                    3. Apparatus as claimed in claim 1 or 2, wherein Y.\n\
                    4. Apparatus as claimed in any of the foregoing claims, wherein Z.\n\
                    5. Apparatus according to any one of claims 2 to 4.\n\
                    6. Method using the apparatus.";
        let blocks = claim_blocks(text);
        assert_eq!(blocks.len(), 6);
        let labels: Vec<_> = blocks.iter().map(|b| b.label.clone().unwrap()).collect();
        assert_eq!(labels, ["1", "2", "3", "4", "5", "6"]);
        assert!(blocks[0].depends_on.is_empty());
        assert_eq!(blocks[1].depends_on, [1]);
        assert_eq!(blocks[2].depends_on, [1, 2]);
        assert_eq!(blocks[3].depends_on, [1, 2, 3]);
        assert!(blocks[3].claim_refs.is_empty());
        assert_eq!(blocks[4].depends_on, [2, 3, 4]);
        assert!(blocks[5].depends_on.is_empty());

        let refs = &blocks[2].claim_refs;
        assert_eq!(refs.len(), 2);
        assert_eq!(slice(text, refs[0].start, refs[0].end), "claim 1");
        assert_eq!(slice(text, refs[1].start, refs[1].end), "2");
        assert_eq!(refs[1].claim, 2);
    }

    #[test]
    fn claimed_and_hyphenated_ranges() {
        let blocks = claim_blocks("1. A.\n2. B.\n3. C.\n4. Device of Claims 1-3 as claimed.");
        assert_eq!(blocks[3].depends_on, [1, 2, 3]);
        assert_eq!(blocks[3].claim_refs.len(), 2);
    }

    #[test]
    fn continuation_lines_belong_to_the_previous_claim() {
        let text = "1. A device comprising:\na housing; and\na lid.\n2. The device of claim 1.";
        let blocks = claim_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            slice(text, blocks[0].start, blocks[0].end),
            "1. A device comprising:\na housing; and\na lid."
        );
        assert_eq!(blocks[1].depends_on, [1]);
    }

    #[test]
    fn a_forward_reference_is_linked_but_not_a_dependency() {
        let blocks = claim_blocks("1. A device for use with the system of claim 2.\n2. A system.");
        assert!(blocks[0].depends_on.is_empty());
        assert_eq!(blocks[0].claim_refs[0].claim, 2);
    }
}
