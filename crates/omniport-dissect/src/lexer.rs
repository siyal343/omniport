#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Number(i64),
    // Keywords & Attributes
    Struct,
    Typedef,
    Attribute,
    Packed,
    Pragma,
    Pack,
    Push,
    Pop,
    // Punctuation
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semicolon,
    Colon,
    Comma,
    // Metadata Comment
    Comment(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

pub struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    _marker: std::marker::PhantomData<&'a str>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            _marker: std::marker::PhantomData,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        if let Some(&ch) = self.chars.get(self.pos) {
            self.pos += 1;
            if ch == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            Some(ch)
        } else {
            None
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();

        while let Some(ch) = self.peek() {
            let line = self.line;
            let col = self.col;

            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    self.advance();
                }
                '/' if self.peek_next() == Some('/') => {
                    // Line comment
                    self.advance();
                    self.advance();
                    let mut comment = String::new();
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        comment.push(c);
                        self.advance();
                    }
                    tokens.push(Token {
                        kind: TokenKind::Comment(comment.trim().to_string()),
                        line,
                        col,
                    });
                }
                '/' if self.peek_next() == Some('*') => {
                    // Block comment
                    self.advance();
                    self.advance();
                    let mut comment = String::new();
                    while let Some(c) = self.peek() {
                        if c == '*' && self.peek_next() == Some('/') {
                            self.advance();
                            self.advance();
                            break;
                        }
                        comment.push(c);
                        self.advance();
                    }
                    tokens.push(Token {
                        kind: TokenKind::Comment(comment.trim().to_string()),
                        line,
                        col,
                    });
                }
                '{' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::LBrace,
                        line,
                        col,
                    });
                }
                '}' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::RBrace,
                        line,
                        col,
                    });
                }
                '(' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::LParen,
                        line,
                        col,
                    });
                }
                ')' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::RParen,
                        line,
                        col,
                    });
                }
                '[' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::LBracket,
                        line,
                        col,
                    });
                }
                ']' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::RBracket,
                        line,
                        col,
                    });
                }
                ';' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Semicolon,
                        line,
                        col,
                    });
                }
                ':' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Colon,
                        line,
                        col,
                    });
                }
                ',' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Comma,
                        line,
                        col,
                    });
                }
                '0'..='9' => {
                    let num = self.read_number()?;
                    tokens.push(Token {
                        kind: TokenKind::Number(num),
                        line,
                        col,
                    });
                }
                'a'..='z' | 'A'..='Z' | '_' | '#' => {
                    let ident = self.read_ident();
                    let kind = match ident.as_str() {
                        "struct" => TokenKind::Struct,
                        "typedef" => TokenKind::Typedef,
                        "__attribute__" | "__attribute" => TokenKind::Attribute,
                        "packed" | "__packed__" | "__packed" => TokenKind::Packed,
                        "#pragma" | "pragma" => TokenKind::Pragma,
                        "pack" => TokenKind::Pack,
                        "push" => TokenKind::Push,
                        "pop" => TokenKind::Pop,
                        _ => TokenKind::Ident(ident),
                    };
                    tokens.push(Token { kind, line, col });
                }
                other => {
                    return Err(format!(
                        "Unexpected character '{}' at line {}, column {}",
                        other, line, col
                    ));
                }
            }
        }

        Ok(tokens)
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_alphanumeric() || ch == '_' || ch == '#' {
                s.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        s
    }

    fn read_number(&mut self) -> Result<i64, String> {
        let mut s = String::new();
        let is_hex = self.peek() == Some('0')
            && (self.peek_next() == Some('x') || self.peek_next() == Some('X'));

        if is_hex {
            s.push(self.advance().unwrap());
            s.push(self.advance().unwrap());
            while let Some(ch) = self.peek() {
                if ch.is_ascii_hexdigit() {
                    s.push(ch);
                    self.advance();
                } else {
                    break;
                }
            }
            i64::from_str_radix(&s[2..], 16)
                .map_err(|e| format!("Invalid hex number '{}': {}", s, e))
        } else {
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    s.push(ch);
                    self.advance();
                } else {
                    break;
                }
            }
            s.parse::<i64>()
                .map_err(|e| format!("Invalid integer '{}': {}", s, e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer_packed_struct() {
        let src = r#"
            struct __attribute__((packed)) Imu {
                uint16_t sync; // scale: 1.0
                int16_t temp;  // scale: 0.01, unit: "C"
                uint8_t flags: 4;
            };
        "#;
        let tokens = Lexer::new(src).tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Struct);
        assert_eq!(tokens[1].kind, TokenKind::Attribute);
        assert_eq!(tokens[2].kind, TokenKind::LParen);
        assert_eq!(tokens[3].kind, TokenKind::LParen);
        assert_eq!(tokens[4].kind, TokenKind::Packed);
    }
}
