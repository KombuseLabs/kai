//! The Mermaid blocks of a Markdown file.
//!
//! A block is a fenced code block whose info string starts with `mermaid`.
//! Fences follow CommonMark: three or more backticks or tildes, indented
//! by at most three spaces, closed by the same character at least as many
//! times. A fence inside another fence is text, so a spec that shows a
//! ```` ```mermaid ```` line inside a ```` ```` ```` block shows no
//! diagram.
//!
//! ```
//! let text = "# Title\n\n```mermaid\nflowchart TD\n  A --> B\n```\n";
//! let blocks = diagrams::markdown::mermaid_blocks(text);
//! assert_eq!(blocks.len(), 1);
//! assert_eq!(blocks[0].fence_line, 3);
//! assert_eq!(blocks[0].first_line, 4);
//! assert_eq!(blocks[0].text, "flowchart TD\n  A --> B\n");
//! ```

/// One Mermaid block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The line of the opening fence, from 1.
    pub fence_line: usize,
    /// The line of the block's first line of text, from 1.
    pub first_line: usize,
    /// The block's text, each line ending in a newline.
    pub text: String,
    /// Whether the closing fence is missing (the block runs to the end).
    pub unclosed: bool,
}

impl Block {
    /// The line in the Markdown file of the block's line `line` (from 1).
    pub fn file_line(&self, line: usize) -> usize {
        self.first_line + line.saturating_sub(1)
    }
}

/// Every Mermaid block of `markdown`, in order.
pub fn mermaid_blocks(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    // The fence that is open: its character, its length, and the block
    // being collected if it is a Mermaid one.
    let mut open: Option<(char, usize, Option<Block>)> = None;
    for (index, line) in markdown.lines().enumerate() {
        let number = index + 1;
        match &mut open {
            None => {
                if let Some((fence, count, info)) = fence(line) {
                    let mermaid = info.split_whitespace().next() == Some("mermaid");
                    let block = mermaid.then(|| Block {
                        fence_line: number,
                        first_line: number + 1,
                        text: String::new(),
                        unclosed: false,
                    });
                    open = Some((fence, count, block));
                }
            }
            Some((fence_char, count, block)) => {
                let closes = fence(line).is_some_and(|(c, n, info)| {
                    c == *fence_char && n >= *count && info.trim().is_empty()
                });
                if closes {
                    if let Some(block) = block.take() {
                        blocks.push(block);
                    }
                    open = None;
                } else if let Some(block) = block {
                    block.text.push_str(line);
                    block.text.push('\n');
                }
            }
        }
    }
    if let Some((_, _, Some(mut block))) = open {
        block.unclosed = true;
        blocks.push(block);
    }
    blocks
}

/// A fence line: its character, how many, and the info string after it.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let first = rest.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = rest.chars().take_while(|&c| c == first).count();
    if count < 3 {
        return None;
    }
    let info = &rest[count..];
    // A backtick fence's info string has no backtick.
    if first == '`' && info.contains('`') {
        return None;
    }
    Some((first, count, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fence_inside_a_fence_is_text() {
        let text =
            "````text\n```mermaid\nflowchart TD\n```\n````\n\n~~~mermaid\nsequenceDiagram\n~~~\n";
        let blocks = mermaid_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].fence_line, 7);
        assert_eq!(blocks[0].text, "sequenceDiagram\n");
    }

    #[test]
    fn other_languages_are_not_diagrams() {
        let text = "```rust\nfn main() {}\n```\n```mermaid   \nflowchart TD\n```\n";
        let blocks = mermaid_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].first_line, 5);
    }

    #[test]
    fn an_unclosed_block_is_marked() {
        let blocks = mermaid_blocks("```mermaid\nflowchart TD\n");
        assert!(blocks[0].unclosed);
        assert_eq!(blocks[0].file_line(1), 2);
    }

    #[test]
    fn a_longer_fence_closes_only_with_as_many() {
        let text = "````mermaid\nflowchart TD\n```\n````\n";
        let blocks = mermaid_blocks(text);
        assert_eq!(blocks[0].text, "flowchart TD\n```\n");
    }
}
