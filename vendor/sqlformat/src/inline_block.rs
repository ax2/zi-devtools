use crate::tokenizer::{Token, TokenKind};

pub(crate) struct BlockInfo {
    length: usize,
    has_forbidden_tokens: bool,
    has_reseved_tokens: bool,
    top_level_token_span: usize,
}

pub(crate) struct InlineBlock {
    level: usize,
    inline_max_length: usize,
    reserved_limit: usize,
    reserved_top_limit: usize,
    info: Vec<BlockInfo>,
}

impl Default for InlineBlock {
    fn default() -> Self {
        InlineBlock {
            info: Vec::new(),
            level: 0,
            inline_max_length: 50,
            reserved_limit: 0,
            reserved_top_limit: 0,
        }
    }
}

impl InlineBlock {
    pub fn new(inline_max_length: usize, reserved_limit: usize, reserved_top_limit: usize) -> Self {
        InlineBlock {
            inline_max_length,
            reserved_limit,
            reserved_top_limit,
            ..Default::default()
        }
    }

    fn is_inline_block(&self, info: &BlockInfo) -> bool {
        !info.has_forbidden_tokens
            && info.length <= self.inline_max_length
            && info.top_level_token_span <= self.reserved_top_limit
            && (!info.has_reseved_tokens || info.length <= self.reserved_limit)
    }

    pub fn begin_if_possible(&mut self, tokens: &[Token<'_>], index: usize) -> bool {
        let require_inline = self.level == 0;
        let Some(info) = self.build_info(tokens, index, require_inline) else {
            return false;
        };
        if require_inline && !self.is_inline_block(&info) {
            return false;
        }
        self.level += 1;
        self.info.push(info);
        true
    }

    pub fn end(&mut self) {
        self.info.pop();
        self.level -= 1;
    }

    pub fn is_active(&self) -> bool {
        self.level > 0
    }

    /// Get the current inline block length
    pub fn cur_len(&self) -> usize {
        self.info.last().map_or(0, |info| info.length)
    }

    fn build_info(
        &self,
        tokens: &[Token<'_>],
        index: usize,
        require_inline: bool,
    ) -> Option<BlockInfo> {
        let mut length = 0;
        let mut level = 0;
        let mut top_level_token_span = 0;
        let mut start_top_level = -1;
        let mut start_span = 0;
        let mut has_forbidden_tokens = false;
        let mut has_reseved_tokens = false;

        for token in &tokens[index..] {
            length += token.value.len();
            // This length cannot fall back under the inline limit. The caller
            // stores no metadata for a block that could not become inline.
            if require_inline && length > self.inline_max_length {
                return None;
            }
            match token.kind {
                TokenKind::ReservedTopLevel | TokenKind::ReservedTopLevelNoIndent => {
                    if start_top_level != -1 {
                        if start_top_level == level {
                            top_level_token_span = top_level_token_span.max(length - start_span);
                            start_top_level = -1;
                        }
                    } else {
                        start_top_level = level;
                        start_span = length;
                    }
                }
                TokenKind::ReservedNewline => {
                    has_reseved_tokens = true;
                }
                TokenKind::OpenParen => {
                    level += 1;
                }
                TokenKind::CloseParen => {
                    level -= 1;
                    if level == 0 {
                        break;
                    }
                }
                _ => {}
            }

            if self.is_forbidden_token(token) {
                has_forbidden_tokens = true;
            }
        }

        // broken syntax let's try our best
        Some(BlockInfo {
            length,
            has_forbidden_tokens,
            has_reseved_tokens,
            top_level_token_span,
        })
    }

    fn is_forbidden_token(&self, token: &Token<'_>) -> bool {
        token.kind == TokenKind::LineComment
            || token.kind == TokenKind::BlockComment
            || token.value == ";"
    }
}
