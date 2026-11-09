use crate::lexer::{Lexer, Token, TokenKind};
use crate::{Endian, FieldDescriptor, FieldKind, PacketSchema, PrimitiveType};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ParseError {
    #[error("Unexpected end of input at line {0}, column {1}")]
    UnexpectedEof(usize, usize),
    #[error("Unexpected token '{0}' at line {1}, column {2}, expected {3}")]
    UnexpectedToken(String, usize, usize, &'static str),
    #[error("Unknown or unsupported C type '{0}' at line {1}, column {2}")]
    UnknownType(String, usize, usize),
    #[error("Invalid array size '{0}' at line {1}, column {2}")]
    InvalidArraySize(String, usize, usize),
    #[error("Invalid bitfield width '{0}' at line {1}, column {2}")]
    InvalidBitfieldWidth(String, usize, usize),
    #[error("Bitfield width {0} exceeds container type capacity {1} bits at line {2}")]
    BitfieldOverflow(usize, usize, usize),
    #[error("No valid struct declaration found in input")]
    NoStructFound,
    #[error("Lexer error: {0}")]
    LexerError(String),
}

struct BitfieldState {
    container_offset: usize,
    container_bytes: usize,
    current_bit: usize,
}

pub struct CStructParser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    endian: Endian,
    _marker: std::marker::PhantomData<&'a str>,
}

impl<'a> CStructParser<'a> {
    pub fn new(source: &'a str, endian: Endian) -> Result<Self, ParseError> {
        let tokens = Lexer::new(source)
            .tokenize()
            .map_err(ParseError::LexerError)?;
        Ok(Self {
            tokens,
            pos: 0,
            endian,
            _marker: std::marker::PhantomData,
        })
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        if self.pos < self.tokens.len() {
            let tok = &self.tokens[self.pos];
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    fn skip_comments(&mut self) {
        while let Some(tok) = self.peek() {
            if matches!(tok.kind, TokenKind::Comment(_)) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn match_token(&mut self, expected: &TokenKind) -> bool {
        self.skip_comments();
        if let Some(tok) = self.peek()
            && &tok.kind == expected
        {
            self.pos += 1;
            return true;
        }
        false
    }

    fn expect_token(
        &mut self,
        expected: &TokenKind,
        desc: &'static str,
    ) -> Result<Token, ParseError> {
        self.skip_comments();
        if let Some(tok) = self.advance() {
            if &tok.kind == expected {
                Ok(tok.clone())
            } else {
                Err(ParseError::UnexpectedToken(
                    format!("{:?}", tok.kind),
                    tok.line,
                    tok.col,
                    desc,
                ))
            }
        } else {
            Err(ParseError::UnexpectedEof(0, 0))
        }
    }

    fn expect_ident(&mut self, desc: &'static str) -> Result<String, ParseError> {
        self.skip_comments();
        if let Some(tok) = self.advance() {
            if let TokenKind::Ident(s) = &tok.kind {
                Ok(s.clone())
            } else {
                Err(ParseError::UnexpectedToken(
                    format!("{:?}", tok.kind),
                    tok.line,
                    tok.col,
                    desc,
                ))
            }
        } else {
            Err(ParseError::UnexpectedEof(0, 0))
        }
    }

    pub fn parse(mut self) -> Result<PacketSchema, ParseError> {
        // Skip any leading pragmas, typedefs, or comments
        let mut struct_name = String::from("AnonymousStruct");

        loop {
            self.skip_comments();
            if self.pos >= self.tokens.len() {
                return Err(ParseError::NoStructFound);
            }

            let tok = self.tokens[self.pos].clone();
            match &tok.kind {
                TokenKind::Pragma => {
                    self.pos += 1;
                    // skip until next line or semicolon or pack directive
                    while let Some(t) = self.peek() {
                        if matches!(t.kind, TokenKind::Struct | TokenKind::Typedef) {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                TokenKind::Typedef => {
                    self.pos += 1;
                }
                TokenKind::Struct => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    self.pos += 1;
                }
            }
        }

        // Check for optional packed attribute or struct name before '{'
        // e.g. struct __attribute__((packed)) Name { ...
        // or struct Name { ...
        while let Some(tok) = self.peek() {
            match &tok.kind {
                TokenKind::Attribute => {
                    self.pos += 1;
                    self.expect_token(&TokenKind::LParen, "(")?;
                    self.expect_token(&TokenKind::LParen, "(")?;
                    self.expect_token(&TokenKind::Packed, "packed")?;
                    self.expect_token(&TokenKind::RParen, ")")?;
                    self.expect_token(&TokenKind::RParen, ")")?;
                }
                TokenKind::Ident(name) => {
                    struct_name = name.clone();
                    self.pos += 1;
                }
                TokenKind::LBrace => {
                    break;
                }
                TokenKind::Comment(_) => {
                    self.pos += 1;
                }
                _ => {
                    break;
                }
            }
        }

        self.expect_token(&TokenKind::LBrace, "{")?;

        let mut fields = Vec::new();
        let mut current_offset: usize = 0;
        let mut bitfield_state: Option<BitfieldState> = None;

        // Parse fields until '}'
        while let Some(tok) = self.peek() {
            if matches!(tok.kind, TokenKind::RBrace) {
                break;
            }
            if matches!(tok.kind, TokenKind::Comment(_)) {
                self.pos += 1;
                continue;
            }

            // Parse field type
            let (ty, is_signed) = self.parse_type()?;

            // Parse field name
            let field_name = self.expect_ident("field identifier")?;

            // Check for array: `[size]`
            let mut array_count = None;
            if self.match_token(&TokenKind::LBracket) {
                let num_tok = self.advance().ok_or(ParseError::UnexpectedEof(0, 0))?;
                let count = match num_tok.kind {
                    TokenKind::Number(n) if n > 0 => n as usize,
                    _ => {
                        return Err(ParseError::InvalidArraySize(
                            format!("{:?}", num_tok.kind),
                            num_tok.line,
                            num_tok.col,
                        ));
                    }
                };
                self.expect_token(&TokenKind::RBracket, "]")?;
                array_count = Some(count);
            }

            // Check for bitfield: `: width`
            let mut bitfield_width = None;
            if self.match_token(&TokenKind::Colon) {
                let num_tok = self.advance().ok_or(ParseError::UnexpectedEof(0, 0))?;
                let width = match num_tok.kind {
                    TokenKind::Number(n) if n > 0 => n as usize,
                    _ => {
                        return Err(ParseError::InvalidBitfieldWidth(
                            format!("{:?}", num_tok.kind),
                            num_tok.line,
                            num_tok.col,
                        ));
                    }
                };
                let max_bits = ty.size_bytes() * 8;
                if width > max_bits {
                    return Err(ParseError::BitfieldOverflow(width, max_bits, num_tok.line));
                }
                bitfield_width = Some(width);
            }

            self.expect_token(&TokenKind::Semicolon, ";")?;

            // Extract metadata from any comment directly following this field
            let mut scale = 1.0;
            let mut unit = None;
            let mut plot = false;

            if let Some(Token {
                kind: TokenKind::Comment(c),
                ..
            }) = self.peek()
            {
                let (meta_scale, meta_unit, meta_plot) = Self::parse_comment_metadata(c);
                scale = meta_scale;
                unit = meta_unit;
                plot = meta_plot;
                self.pos += 1;
            }

            // Calculate field layout
            if let Some(w) = bitfield_width {
                let container_bytes = ty.size_bytes();
                let max_bits = container_bytes * 8;

                match bitfield_state {
                    Some(ref mut s)
                        if s.container_bytes == container_bytes
                            && s.current_bit + w <= max_bits =>
                    {
                        let bit_offset = s.current_bit as u8;
                        fields.push(FieldDescriptor {
                            name: field_name,
                            offset: s.container_offset,
                            ty,
                            scale,
                            unit,
                            plot_telemetry: plot,
                            kind: FieldKind::Bitfield {
                                bit_offset,
                                bit_width: w as u8,
                                signed: is_signed,
                            },
                        });
                        s.current_bit += w;
                    }
                    _ => {
                        // Flush previous bitfield if needed
                        if let Some(s) = bitfield_state.take() {
                            let bytes_used = s.current_bit.div_ceil(8);
                            current_offset = s.container_offset + bytes_used;
                        }

                        let bit_offset = 0u8;
                        fields.push(FieldDescriptor {
                            name: field_name,
                            offset: current_offset,
                            ty,
                            scale,
                            unit,
                            plot_telemetry: plot,
                            kind: FieldKind::Bitfield {
                                bit_offset,
                                bit_width: w as u8,
                                signed: is_signed,
                            },
                        });

                        bitfield_state = Some(BitfieldState {
                            container_offset: current_offset,
                            container_bytes,
                            current_bit: w,
                        });
                    }
                }
            } else {
                // Non-bitfield: flush bitfield container if active
                if let Some(s) = bitfield_state.take() {
                    let bytes_used = s.current_bit.div_ceil(8);
                    current_offset = s.container_offset + bytes_used;
                }

                if let Some(count) = array_count {
                    fields.push(FieldDescriptor {
                        name: field_name,
                        offset: current_offset,
                        ty,
                        scale,
                        unit,
                        plot_telemetry: plot,
                        kind: FieldKind::Array { count },
                    });
                    current_offset += ty.size_bytes() * count;
                } else {
                    fields.push(FieldDescriptor {
                        name: field_name,
                        offset: current_offset,
                        ty,
                        scale,
                        unit,
                        plot_telemetry: plot,
                        kind: FieldKind::Scalar,
                    });
                    current_offset += ty.size_bytes();
                }
            }
        }

        self.expect_token(&TokenKind::RBrace, "}")?;

        // Flush trailing bitfield
        if let Some(s) = bitfield_state.take() {
            let bytes_used = s.current_bit.div_ceil(8);
            current_offset = s.container_offset + bytes_used;
        }

        // Optional typedef trailing alias or attributes
        // e.g. } AliasName; or } __attribute__((packed));
        while let Some(tok) = self.peek() {
            match &tok.kind {
                TokenKind::Attribute => {
                    self.pos += 1;
                    self.expect_token(&TokenKind::LParen, "(")?;
                    self.expect_token(&TokenKind::LParen, "(")?;
                    self.expect_token(&TokenKind::Packed, "packed")?;
                    self.expect_token(&TokenKind::RParen, ")")?;
                    self.expect_token(&TokenKind::RParen, ")")?;
                }
                TokenKind::Ident(alias) => {
                    struct_name = alias.clone();
                    self.pos += 1;
                }
                TokenKind::Semicolon => {
                    self.pos += 1;
                    break;
                }
                TokenKind::Comment(_) => {
                    self.pos += 1;
                }
                _ => break,
            }
        }

        Ok(PacketSchema {
            name: struct_name,
            endian: self.endian,
            total_size: current_offset,
            fields,
        })
    }

    fn parse_type(&mut self) -> Result<(PrimitiveType, bool), ParseError> {
        let tok = self.advance().ok_or(ParseError::UnexpectedEof(0, 0))?;
        let ident = match &tok.kind {
            TokenKind::Ident(s) => s.as_str(),
            other => {
                return Err(ParseError::UnexpectedToken(
                    format!("{:?}", other),
                    tok.line,
                    tok.col,
                    "type specifier",
                ));
            }
        };

        match ident {
            "uint8_t" | "u8" | "unsigned char" => Ok((PrimitiveType::U8, false)),
            "int8_t" | "i8" | "signed char" => Ok((PrimitiveType::I8, true)),
            "char" => Ok((PrimitiveType::U8, false)),
            "bool" | "_Bool" => Ok((PrimitiveType::U8, false)),
            "uint16_t" | "u16" => Ok((PrimitiveType::U16, false)),
            "int16_t" | "i16" => Ok((PrimitiveType::I16, true)),
            "uint32_t" | "u32" => Ok((PrimitiveType::U32, false)),
            "int32_t" | "i32" => Ok((PrimitiveType::I32, true)),
            "uint64_t" | "u64" => Ok((PrimitiveType::U64, false)),
            "int64_t" | "i64" => Ok((PrimitiveType::I64, true)),
            "float" | "f32" => Ok((PrimitiveType::F32, true)),
            "double" | "f64" => Ok((PrimitiveType::F64, true)),
            "unsigned" => {
                // Check if next token is short, int, long, etc.
                if let Some(Token {
                    kind: TokenKind::Ident(sub),
                    ..
                }) = self.peek()
                {
                    match sub.as_str() {
                        "char" => {
                            self.pos += 1;
                            Ok((PrimitiveType::U8, false))
                        }
                        "short" => {
                            self.pos += 1;
                            Ok((PrimitiveType::U16, false))
                        }
                        "int" | "long" => {
                            self.pos += 1;
                            Ok((PrimitiveType::U32, false))
                        }
                        _ => Ok((PrimitiveType::U32, false)),
                    }
                } else {
                    Ok((PrimitiveType::U32, false))
                }
            }
            "signed" => {
                if let Some(Token {
                    kind: TokenKind::Ident(sub),
                    ..
                }) = self.peek()
                {
                    match sub.as_str() {
                        "char" => {
                            self.pos += 1;
                            Ok((PrimitiveType::I8, true))
                        }
                        "short" => {
                            self.pos += 1;
                            Ok((PrimitiveType::I16, true))
                        }
                        "int" | "long" => {
                            self.pos += 1;
                            Ok((PrimitiveType::I32, true))
                        }
                        _ => Ok((PrimitiveType::I32, true)),
                    }
                } else {
                    Ok((PrimitiveType::I32, true))
                }
            }
            "short" => Ok((PrimitiveType::I16, true)),
            "int" | "long" => Ok((PrimitiveType::I32, true)),
            other => Err(ParseError::UnknownType(
                other.to_string(),
                tok.line,
                tok.col,
            )),
        }
    }

    /// Extracts scale, unit, and plot attributes from comments like:
    /// `// scale: 0.01, unit: "degC", plot: true`
    fn parse_comment_metadata(comment: &str) -> (f64, Option<String>, bool) {
        let mut scale = 1.0;
        let mut unit = None;
        let mut plot = false;

        for part in comment.split([',', ';']) {
            let part = part.trim();
            if let Some(idx) = part.find(':').or_else(|| part.find('=')) {
                let key = part[..idx].trim().to_lowercase();
                let val = part[idx + 1..].trim();

                match key.as_str() {
                    "scale" | "@scale" => {
                        if let Ok(s) = val.parse::<f64>() {
                            scale = s;
                        }
                    }
                    "unit" | "@unit" => {
                        let clean_val = val.trim_matches('"').trim_matches('\'').trim();
                        unit = Some(clean_val.to_string());
                    }
                    "plot" | "@plot" | "plot_telemetry" => {
                        plot = val.eq_ignore_ascii_case("true") || val == "1";
                    }
                    _ => {}
                }
            } else if part.eq_ignore_ascii_case("plot") || part.eq_ignore_ascii_case("@plot") {
                plot = true;
            }
        }

        (scale, unit, plot)
    }
}
