//! Hand-rolled lexer -- no external crate needed for a grammar this small.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Struct,
    Let,
    Set,
    Intent,
    Bound,
    As,
    Print,
    True,
    False,
    Default,
    Resolve,
    Above,
    Else,
    Arrow,    // ->
    FatArrow, // =>
    Le,       // <=
    Ge,       // >=
    LBrace,
    RBrace,
    LParen,
    RParen,
    LAngle, // <
    RAngle, // >
    Colon,
    Comma,
    Dot,
    Equals,
    Ident(String),
    Number(f64),
    Str(String),
    Newline,
    Eof,
}

/// A token paired with the (1-based) source line it started on -- threaded
/// through the parser so parse errors, and eventually the LSP, can point at
/// a real location instead of "somewhere in the file".
pub type Spanned = (Token, usize);

#[derive(Debug)]
pub struct LexError {
    pub line: usize,
    pub message: String,
}

pub fn lex(src: &str) -> Result<Vec<Spanned>, LexError> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens: Vec<Spanned> = Vec::new();
    let mut i = 0;
    let mut line = 1usize;
    let mut depth: i32 = 0; // suppress NEWLINE while inside (), {}, or <>

    while i < chars.len() {
        let c = chars[i];

        if c == '\n' {
            if depth == 0 {
                tokens.push((Token::Newline, line));
            }
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '"' {
            let start_line = line;
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    s.push(chars[i + 1]);
                    i += 2;
                } else {
                    s.push(chars[i]);
                    i += 1;
                }
            }
            if i >= chars.len() {
                return Err(LexError { line: start_line, message: "unterminated string".into() });
            }
            i += 1; // closing quote
            tokens.push((Token::Str(s), start_line));
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let value: f64 = text.parse().map_err(|_| LexError {
                line,
                message: format!("bad number literal {text:?}"),
            })?;
            tokens.push((Token::Number(value), line));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let tok = match text.as_str() {
                "struct" => Token::Struct,
                "let" => Token::Let,
                "set" => Token::Set,
                "intent" => Token::Intent,
                "bound" => Token::Bound,
                "as" => Token::As,
                "print" => Token::Print,
                "true" => Token::True,
                "false" => Token::False,
                "default" => Token::Default,
                "resolve" => Token::Resolve,
                "above" => Token::Above,
                "else" => Token::Else,
                _ => Token::Ident(text),
            };
            tokens.push((tok, line));
            continue;
        }

        // two-character operators
        if c == '-' && chars.get(i + 1) == Some(&'>') {
            tokens.push((Token::Arrow, line));
            i += 2;
            continue;
        }
        if c == '=' && chars.get(i + 1) == Some(&'>') {
            tokens.push((Token::FatArrow, line));
            i += 2;
            continue;
        }
        if c == '<' && chars.get(i + 1) == Some(&'=') {
            tokens.push((Token::Le, line));
            i += 2;
            continue;
        }
        if c == '>' && chars.get(i + 1) == Some(&'=') {
            tokens.push((Token::Ge, line));
            i += 2;
            continue;
        }

        // single-character punctuation
        let tok = match c {
            '{' => { depth += 1; Token::LBrace }
            '}' => { depth -= 1; Token::RBrace }
            '(' => { depth += 1; Token::LParen }
            ')' => { depth -= 1; Token::RParen }
            '<' => { depth += 1; Token::LAngle }
            '>' => { depth -= 1; Token::RAngle }
            ':' => Token::Colon,
            ',' => Token::Comma,
            '.' => Token::Dot,
            '=' => Token::Equals,
            other => {
                return Err(LexError {
                    line,
                    message: format!("unexpected character {other:?}"),
                })
            }
        };
        tokens.push((tok, line));
        i += 1;
    }

    tokens.push((Token::Eof, line));
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Token> {
        lex(src).unwrap().into_iter().map(|(t, _)| t).collect()
    }

    #[test]
    fn lexes_keywords_and_punct() {
        assert_eq!(
            kinds("struct User { age: int }"),
            vec![
                Token::Struct,
                Token::Ident("User".into()),
                Token::LBrace,
                Token::Ident("age".into()),
                Token::Colon,
                Token::Ident("int".into()),
                Token::RBrace,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lexes_generic_and_arrows() {
        let toks = kinds("Probability<float> -> x => y <= 1 >= 2");
        assert!(toks.contains(&Token::LAngle));
        assert!(toks.contains(&Token::RAngle));
        assert!(toks.contains(&Token::Arrow));
        assert!(toks.contains(&Token::FatArrow));
        assert!(toks.contains(&Token::Le));
        assert!(toks.contains(&Token::Ge));
    }

    #[test]
    fn suppresses_newlines_inside_braces() {
        let toks = kinds("{\n\"a\": 1\n}\nprint(1)");
        // one Newline: after the closing brace, before `print`; none inside.
        let count = toks.iter().filter(|t| **t == Token::Newline).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn tracks_line_numbers_across_newlines() {
        let toks = lex("let a = 1\nlet b = 2").unwrap();
        let b_line = toks.iter().find(|(t, _)| *t == Token::Ident("b".into())).unwrap().1;
        assert_eq!(b_line, 2);
    }
}
