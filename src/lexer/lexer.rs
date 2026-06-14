use crate::lexer::{FloatSuffix, IntegerSuffix, LexError, Span, Token, TokenKind};

pub(crate) struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace_and_comments();
            if self.pos >= self.bytes.len() {
                let span = self.span_at(self.pos, self.pos, self.line, self.col);
                tokens.push(Token::new(TokenKind::Eof, span));
                return Ok(tokens);
            }
            tokens.push(self.lex_token()?);
        }
    }

    fn lex_token(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.col;
        let c = self.bytes[self.pos];
        match c {
            b'0'..=b'9' => self.lex_number(),
            b'"' => self.lex_string(),
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => Ok(self.lex_ident_or_keyword()),
            b';' => Ok(self.single_char(start, start_line, start_col, TokenKind::Semicolon)),
            b':' => Ok(self.single_char(start, start_line, start_col, TokenKind::Colon)),
            b',' => Ok(self.single_char(start, start_line, start_col, TokenKind::Comma)),
            b'.' => Ok(self.dot_or_range(start, start_line, start_col)),
            b'(' => Ok(self.single_char(start, start_line, start_col, TokenKind::LParen)),
            b')' => Ok(self.single_char(start, start_line, start_col, TokenKind::RParen)),
            b'{' => Ok(self.single_char(start, start_line, start_col, TokenKind::LBrace)),
            b'}' => Ok(self.single_char(start, start_line, start_col, TokenKind::RBrace)),
            b'[' => Ok(self.single_char(start, start_line, start_col, TokenKind::LBracket)),
            b']' => Ok(self.single_char(start, start_line, start_col, TokenKind::RBracket)),
            b'+' => Ok(self.plus_or_assign(start, start_line, start_col)),
            b'-' => Ok(self.minus_or_arrow(start, start_line, start_col)),
            b'*' => Ok(self.star_or_assign(start, start_line, start_col)),
            b'/' => Ok(self.slash_or_assign(start, start_line, start_col)),
            b'%' => Ok(self.percent_or_assign(start, start_line, start_col)),
            b'=' => Ok(self.eq_or_eqeq(start, start_line, start_col)),
            b'!' => Ok(self.bang_or_bangeq(start, start_line, start_col)),
            b'<' => Ok(self.lt_chain(start, start_line, start_col)),
            b'>' => Ok(self.gt_chain(start, start_line, start_col)),
            b'&' => Ok(self.amp_chain(start, start_line, start_col)),
            b'|' => Ok(self.pipe_chain(start, start_line, start_col)),
            b'^' => Ok(self.caret_or_assign(start, start_line, start_col)),
            _ => {
                self.bump();
                Err(LexError {
                    message: format!("unexpected character `{}`", c as char),
                    span: self.span_at(start, self.pos, start_line, start_col),
                })
            }
        }
    }

    // ---- Numbers ----

    fn lex_number(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.col;

        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos] == b'0' {
            match self.bytes[self.pos + 1] {
                b'x' | b'X' => return self.lex_radix(start, start_line, start_col, 16, "hex"),
                b'o' | b'O' => return self.lex_radix(start, start_line, start_col, 8, "octal"),
                b'b' | b'B' => return self.lex_radix(start, start_line, start_col, 2, "binary"),
                _ => {}
            }
        }

        self.scan_digits();
        let mut is_float = self.try_dot_digit();
        if is_float {
            self.scan_digits();
        }
        if self.try_char_e() {
            self.try_sign();
            let exp_start = self.pos;
            self.scan_digits();
            if self.pos == exp_start {
                return Err(LexError {
                    message: "missing digits after exponent in float literal".into(),
                    span: self.span_at(start, self.pos, start_line, start_col),
                });
            }
            is_float = true;
        }
        self.try_integer_suffix(start, start_line, start_col, is_float)
    }

    fn is_valid_digit(c: u8, radix: u32) -> bool {
        match radix {
            2 => c == b'0' || c == b'1',
            8 => (b'0'..=b'7').contains(&c),
            16 => c.is_ascii_hexdigit(),
            _ => false,
        }
    }

    fn lex_radix(
        &mut self,
        start: usize,
        start_line: usize,
        start_col: usize,
        radix: u32,
        name: &str,
    ) -> Result<Token, LexError> {
        self.bump(); // '0'
        self.bump(); // 'x'/'o'/'b'
        let digits_start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_hexdigit() {
            self.bump();
        }
        if self.pos == digits_start {
            return Err(LexError {
                message: format!("empty {} literal", name),
                span: self.span_at(start, self.pos, start_line, start_col),
            });
        }
        let raw = self.bytes_to_str(&self.bytes[digits_start..self.pos], start, start_line, start_col)?;
        for &c in raw.as_bytes() {
            if !Self::is_valid_digit(c, radix) {
                return Err(LexError {
                    message: format!("invalid digit `{}` in {} literal", c as char, name),
                    span: self.span_at(start, self.pos, start_line, start_col),
                });
            }
        }
        let val = i64::from_str_radix(raw, radix).map_err(|e| LexError {
            message: match e.kind() {
                std::num::IntErrorKind::PosOverflow | std::num::IntErrorKind::NegOverflow =>
                    format!("{} literal overflow: `{}`", name, raw),
                _ => format!("invalid {} literal `{}`", name, raw),
            },
            span: self.span_at(start, self.pos, start_line, start_col),
        })?;
        let end = self.pos;
        let suffix = self.lex_integer_suffix_name();
        Ok(Token::new(
            TokenKind::Int(val, suffix),
            self.span_at(start, end, start_line, start_col),
        ))
    }

    fn try_integer_suffix(
        &mut self,
        start: usize,
        start_line: usize,
        start_col: usize,
        is_float: bool,
    ) -> Result<Token, LexError> {
        let end;
        let span;
        if is_float {
            let suffix = self.lex_float_suffix_name();
            end = self.pos;
            span = self.span_at(start, end, start_line, start_col);
            let slice = &self.bytes[start..end];
            let raw = self.bytes_to_str(slice, start, start_line, start_col)?;
            let suffix_len = float_suffix_str_len(suffix);
            let num_raw = &raw[..raw.len().saturating_sub(suffix_len)];
            let val: f64 = num_raw.parse().map_err(|_| LexError {
                message: format!("invalid float literal `{}`", num_raw),
                span,
            })?;
            Ok(Token::new(TokenKind::Float(val, suffix), span))
        } else {
            let suffix = self.lex_integer_suffix_name();
            end = self.pos;
            span = self.span_at(start, end, start_line, start_col);
            let slice = &self.bytes[start..end];
            let raw = self.bytes_to_str(slice, start, start_line, start_col)?;
            let suffix_len = integer_suffix_str_len(suffix);
            let num_raw = &raw[..raw.len().saturating_sub(suffix_len)];
            let val: i64 = num_raw.parse().map_err(|_| LexError {
                message: format!("invalid integer literal `{}`", num_raw),
                span,
            })?;
            Ok(Token::new(TokenKind::Int(val, suffix), span))
        }
    }

    fn lex_integer_suffix_name(&mut self) -> IntegerSuffix {
        let save = (self.pos, self.col, self.line);
        let name = self.scan_ident();
        match name.as_str() {
            "i8" => IntegerSuffix::I8,
            "i16" => IntegerSuffix::I16,
            "i32" => IntegerSuffix::I32,
            "i64" => IntegerSuffix::I64,
            "u8" => IntegerSuffix::U8,
            "u16" => IntegerSuffix::U16,
            "u32" => IntegerSuffix::U32,
            "u64" => IntegerSuffix::U64,
            _ => {
                self.pos = save.0;
                self.col = save.1;
                self.line = save.2;
                IntegerSuffix::None
            }
        }
    }

    fn lex_float_suffix_name(&mut self) -> FloatSuffix {
        let save = (self.pos, self.col, self.line);
        let name = self.scan_ident();
        match name.as_str() {
            "f32" => FloatSuffix::F32,
            "f64" => FloatSuffix::F64,
            _ => {
                self.pos = save.0;
                self.col = save.1;
                self.line = save.2;
                FloatSuffix::None
            }
        }
    }

    // ---- String ----

    fn lex_string(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.col;
        self.bump();
        let mut buf = String::new();
        loop {
            if self.pos >= self.bytes.len() {
                return Err(LexError {
                    message: "unterminated string literal".into(),
                    span: self.span_at(start, self.pos, start_line, start_col),
                });
            }
            let c = self.bytes[self.pos];
            if c == b'"' {
                self.bump();
                break;
            }
            if c == b'\\' {
                self.bump();
                if self.pos >= self.bytes.len() {
                    return Err(LexError {
                        message: "unterminated escape sequence".into(),
                        span: self.span_at(start, self.pos, start_line, start_col),
                    });
                }
                match self.bytes[self.pos] {
                    b'0' => buf.push('\0'),
                    b'n' => buf.push('\n'),
                    b't' => buf.push('\t'),
                    b'r' => buf.push('\r'),
                    b'\\' => buf.push('\\'),
                    b'"' => buf.push('"'),
                    c => {
                        return Err(LexError {
                            message: format!("invalid escape sequence `\\{}`", c as char),
                            span: self.span_at(self.pos - 1, self.pos + 1, self.line, self.col),
                        });
                    }
                }
                self.bump();
            } else if c >= 0x80 {
                // Multi-byte UTF-8: collect the run of non-ASCII bytes
                // (up to the next ASCII byte or end) and validate as UTF-8.
                let run_start = self.pos;
                let mut run_end = run_start;
                while run_end < self.bytes.len() && self.bytes[run_end] >= 0x80 {
                    run_end += 1;
                }
                let chunk = std::str::from_utf8(&self.bytes[run_start..run_end])
                    .map_err(|e| LexError {
                        message: format!(
                            "invalid UTF-8 in string literal: {}",
                            e
                        ),
                        span: self.span_at(run_start, run_end, self.line, self.col),
                    })?;
                self.col += chunk.chars().count();
                self.pos = run_end;
                buf.push_str(chunk);
            } else {
                if c == b'\n' {
                    self.line += 1;
                    self.col = 0;
                }
                buf.push(c as char);
                self.bump();
            }
        }
        Ok(Token::new(
            TokenKind::String(buf),
            self.span_at(start, self.pos, start_line, start_col),
        ))
    }

    // ---- Identifiers & keywords ----

    fn lex_ident_or_keyword(&mut self) -> Token {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.col;
        let raw = self.scan_ident();
        let end = self.pos;
        let kind = match raw.as_str() {
            "let" => TokenKind::Let,
            "def" => TokenKind::Fn,
            "model" => TokenKind::Struct,

            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "i8" => TokenKind::I8,
            "i16" => TokenKind::I16,
            "i32" => TokenKind::I32,
            "i64" => TokenKind::I64,
            "u8" => TokenKind::U8,
            "u16" => TokenKind::U16,
            "u32" => TokenKind::U32,
            "u64" => TokenKind::U64,
            "f32" => TokenKind::F32,
            "f64" => TokenKind::F64,
            "string" => TokenKind::StringTy,
            "bool" => TokenKind::Bool,
            "void" => TokenKind::Void,
            _ => TokenKind::Ident(raw),
        };
        Token::new(kind, self.span_at(start, end, start_line, start_col))
    }

    // ---- Single/multi-char punctuation ----

    fn single_char(
        &mut self,
        start: usize,
        start_line: usize,
        start_col: usize,
        kind: TokenKind,
    ) -> Token {
        self.bump();
        Token::new(kind, self.span_at(start, self.pos, start_line, start_col))
    }

    fn dot_or_range(&mut self, start: usize, line: usize, col: usize) -> Token {
        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'.' {
            self.bump();
            self.bump();
            Token::new(TokenKind::DotDot, self.span_at(start, self.pos, line, col))
        } else {
            self.bump();
            Token::new(TokenKind::Dot, self.span_at(start, self.pos, line, col))
        }
    }

    fn plus_or_assign(&mut self, start: usize, line: usize, col: usize) -> Token {
        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'+' {
            self.bump();
            self.bump();
            Token::new(TokenKind::PlusPlus, self.span_at(start, self.pos, line, col))
        } else {
            self.bump_or_assign(start, line, col, TokenKind::Plus, TokenKind::PlusEq)
        }
    }

    fn minus_or_arrow(&mut self, start: usize, line: usize, col: usize) -> Token {
        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'-' {
            self.bump();
            self.bump();
            Token::new(TokenKind::MinusMinus, self.span_at(start, self.pos, line, col))
        } else if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'>' {
            self.bump();
            self.bump();
            Token::new(TokenKind::Arrow, self.span_at(start, self.pos, line, col))
        } else {
            self.bump_or_assign(start, line, col, TokenKind::Minus, TokenKind::MinusEq)
        }
    }

    fn star_or_assign(&mut self, start: usize, line: usize, col: usize) -> Token {
        self.bump_or_assign(start, line, col, TokenKind::Star, TokenKind::StarEq)
    }

    fn slash_or_assign(&mut self, start: usize, line: usize, col: usize) -> Token {
        self.bump_or_assign(start, line, col, TokenKind::Slash, TokenKind::SlashEq)
    }

    fn percent_or_assign(&mut self, start: usize, line: usize, col: usize) -> Token {
        self.bump_or_assign(start, line, col, TokenKind::Percent, TokenKind::PercentEq)
    }

    fn eq_or_eqeq(&mut self, start: usize, line: usize, col: usize) -> Token {
        self.bump_or_assign(start, line, col, TokenKind::Eq, TokenKind::EqEq)
    }

    fn bang_or_bangeq(&mut self, start: usize, line: usize, col: usize) -> Token {
        self.bump_or_assign(start, line, col, TokenKind::Bang, TokenKind::BangEq)
    }

    fn lt_chain(&mut self, start: usize, line: usize, col: usize) -> Token {
        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'<' {
            self.bump();
            self.bump();
            let tok = if self.pos < self.bytes.len() && self.bytes[self.pos] == b'=' {
                self.bump();
                TokenKind::LtLtEq
            } else {
                TokenKind::LtLt
            };
            Token::new(tok, self.span_at(start, self.pos, line, col))
        } else {
            self.bump_or_assign(start, line, col, TokenKind::Lt, TokenKind::LtEq)
        }
    }

    fn gt_chain(&mut self, start: usize, line: usize, col: usize) -> Token {
        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'>' {
            self.bump();
            self.bump();
            let tok = if self.pos < self.bytes.len() && self.bytes[self.pos] == b'=' {
                self.bump();
                TokenKind::GtGtEq
            } else {
                TokenKind::GtGt
            };
            Token::new(tok, self.span_at(start, self.pos, line, col))
        } else {
            self.bump_or_assign(start, line, col, TokenKind::Gt, TokenKind::GtEq)
        }
    }

    fn amp_chain(&mut self, start: usize, line: usize, col: usize) -> Token {
        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'&' {
            self.bump();
            self.bump();
            Token::new(TokenKind::AmpAmp, self.span_at(start, self.pos, line, col))
        } else {
            self.bump_or_assign(start, line, col, TokenKind::Amp, TokenKind::AmpEq)
        }
    }

    fn pipe_chain(&mut self, start: usize, line: usize, col: usize) -> Token {
        if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'|' {
            self.bump();
            self.bump();
            Token::new(
                TokenKind::PipePipe,
                self.span_at(start, self.pos, line, col),
            )
        } else {
            self.bump_or_assign(start, line, col, TokenKind::Pipe, TokenKind::PipeEq)
        }
    }

    fn caret_or_assign(&mut self, start: usize, line: usize, col: usize) -> Token {
        self.bump_or_assign(start, line, col, TokenKind::Caret, TokenKind::CaretEq)
    }

    fn bump_or_assign(
        &mut self,
        start: usize,
        line: usize,
        col: usize,
        one: TokenKind,
        two: TokenKind,
    ) -> Token {
        self.bump();
        if self.pos < self.bytes.len() && self.bytes[self.pos] == b'=' {
            self.bump();
            Token::new(two, self.span_at(start, self.pos, line, col))
        } else {
            Token::new(one, self.span_at(start, self.pos, line, col))
        }
    }

    // ---- Helpers ----

    fn scan_digits(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.bump();
        }
    }

    fn try_dot_digit(&mut self) -> bool {
        if self.pos + 1 < self.bytes.len()
            && self.bytes[self.pos] == b'.'
            && self.bytes[self.pos + 1].is_ascii_digit()
        {
            self.bump();
            true
        } else {
            false
        }
    }

    fn try_char_e(&mut self) -> bool {
        if self.pos < self.bytes.len()
            && (self.bytes[self.pos] == b'e' || self.bytes[self.pos] == b'E')
        {
            self.bump();
            true
        } else {
            false
        }
    }

    fn try_sign(&mut self) {
        if self.pos < self.bytes.len()
            && (self.bytes[self.pos] == b'+' || self.bytes[self.pos] == b'-')
        {
            self.bump();
        }
    }

    fn scan_ident(&mut self) -> String {
        let start = self.pos;
        let start_col = self.col;
        let start_line = self.line;
        while self.pos < self.bytes.len()
            && (self.bytes[self.pos].is_ascii_alphanumeric() || self.bytes[self.pos] == b'_')
        {
            self.bump();
        }
        match self.bytes_to_str(&self.bytes[start..self.pos], start, start_line, start_col) {
            Ok(s) => s.to_string(),
            // scan_ident never sees non-ASCII because the byte-level
            // predicate above only accepts ASCII alphanumeric / `_`.
            // Fall back to a lossy conversion so a future regression
            // in the predicate doesn't crash the compiler.
            Err(_) => String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned(),
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            if self.pos >= self.bytes.len() {
                return;
            }
            match self.bytes[self.pos] {
                b' ' | b'\t' => {
                    self.bump();
                }
                b'\n' => {
                    self.line += 1;
                    self.col = 1;
                    self.pos += 1;
                }
                b'\r' => {
                    self.line += 1;
                    self.col = 1;
                    self.pos += 1;
                    if self.pos < self.bytes.len() && self.bytes[self.pos] == b'\n' {
                        self.pos += 1;
                    }
                }
                b'/' if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'/' => {
                    while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                b'/' if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'*' => {
                    self.bump();
                    self.bump();
                    loop {
                        if self.pos >= self.bytes.len() {
                            return;
                        }
                        if self.bytes[self.pos] == b'*'
                            && self.pos + 1 < self.bytes.len()
                            && self.bytes[self.pos + 1] == b'/'
                        {
                            self.bump();
                            self.bump();
                            break;
                        }
                        if self.bytes[self.pos] == b'\n' {
                            self.line += 1;
                            self.col = 1;
                            self.pos += 1;
                        } else if self.bytes[self.pos] == b'\r' {
                            self.line += 1;
                            self.col = 1;
                            self.pos += 1;
                            if self.pos < self.bytes.len() && self.bytes[self.pos] == b'\n' {
                                self.pos += 1;
                            }
                        } else {
                            self.bump();
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn bump(&mut self) {
        self.pos += 1;
        self.col += 1;
    }

    fn span_at(&self, start: usize, end: usize, line: usize, col: usize) -> Span {
        Span::new(start, end, line, col)
    }

    /// Convert a byte slice to `&str` or return a `LexError` pointing
    /// at the supplied span. The caller is expected to have produced
    /// the slice by scanning ASCII-only characters (digits, hex digits,
    /// identifier chars, …), so a UTF-8 error indicates an internal
    /// lexer bug — but we surface it as a proper error rather than
    /// panicking, matching how `lex_string` already handles non-ASCII
    /// bytes.
    fn bytes_to_str<'b>(&self, bytes: &'b [u8], start: usize, start_line: usize, start_col: usize) -> Result<&'b str, LexError> {
        std::str::from_utf8(bytes).map_err(|e| LexError {
            message: format!("invalid UTF-8 in lexer buffer: {}", e),
            span: self.span_at(start, start + bytes.len(), start_line, start_col),
        })
    }
}

fn integer_suffix_str_len(s: IntegerSuffix) -> usize {
    match s {
        IntegerSuffix::None => 0,
        IntegerSuffix::I8 | IntegerSuffix::U8 => 2,
        IntegerSuffix::I16 | IntegerSuffix::U16
        | IntegerSuffix::I32 | IntegerSuffix::U32
        | IntegerSuffix::I64 | IntegerSuffix::U64 => 3,
    }
}

fn float_suffix_str_len(s: FloatSuffix) -> usize {
    match s {
        FloatSuffix::None => 0,
        FloatSuffix::F32 | FloatSuffix::F64 => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_valid_utf8_string() {
        let mut lexer = Lexer::new("\"café\"");
        let tokens = lexer.tokenize().unwrap();
        match &tokens[0].kind {
            TokenKind::String(s) => assert_eq!(s, "café"),
            other => panic!("expected String token, got {:?}", other),
        }
    }

    #[test]
    fn rejects_lone_utf8_lead_byte() {
        // 0xC3 starts a 2-byte UTF-8 sequence; without a continuation
        // byte, this is an encoding error and must not be silently
        // accepted as U+00C3 ('Ã').
        let bytes: Vec<u8> = {
            let mut v = Vec::new();
            v.push(b'"');
            v.extend_from_slice(b"ok");
            v.push(0xC3);
            v.push(b'"');
            v
        };
        let source = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let mut lexer = Lexer::new(source);
        assert!(lexer.tokenize().is_err());
    }
}
