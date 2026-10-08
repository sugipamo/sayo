use crate::error::{Error, Result};
use crate::external::ExternalReference;
use crate::model::Expression;

pub fn parse(source: &str) -> Result<Expression> {
    let tokens = tokenize(source)?;
    if tokens.is_empty() {
        return Err(Error::Parse("empty expression".into()));
    }
    let (expression, position) = parse_sequence(&tokens, 0, false)?;
    if position != tokens.len() {
        return Err(Error::Parse(format!(
            "unexpected token: {}",
            tokens[position].description()
        )));
    }
    Ok(expression)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Atom(String),
    StringLiteral(String),
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
    OpenAngle,
    CloseAngle,
    Comma,
}

impl Token {
    fn description(&self) -> String {
        match self {
            Self::Atom(value) => value.clone(),
            Self::StringLiteral(_) => "string literal".into(),
            Self::OpenParen => "(".into(),
            Self::CloseParen => ")".into(),
            Self::OpenBrace => "{".into(),
            Self::CloseBrace => "}".into(),
            Self::OpenAngle => "<".into(),
            Self::CloseAngle => ">".into(),
            Self::Comma => ",".into(),
        }
    }
}

fn tokenize(source: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut characters = source.chars();
    while let Some(character) = characters.next() {
        if character.is_whitespace() {
            push_current(&mut tokens, &mut current);
        } else if character == '"' {
            push_current(&mut tokens, &mut current);
            let mut value = String::new();
            loop {
                match characters.next() {
                    Some('"') => break,
                    Some('\\') => match characters.next() {
                        Some('"') => value.push('"'),
                        Some('\\') => value.push('\\'),
                        Some('n') => value.push('\n'),
                        Some('r') => value.push('\r'),
                        Some('t') => value.push('\t'),
                        Some(escape) => {
                            return Err(Error::Parse(format!("unknown string escape: \\{escape}")));
                        }
                        None => return Err(Error::Parse("unclosed string literal".into())),
                    },
                    Some('\n' | '\r') => {
                        return Err(Error::Parse(
                            "direct newline in string literal is not allowed".into(),
                        ));
                    }
                    Some(character) => value.push(character),
                    None => return Err(Error::Parse("unclosed string literal".into())),
                }
            }
            tokens.push(Token::StringLiteral(value));
        } else if matches!(character, '(' | ')' | '{' | '}' | '<' | '>' | ',') {
            push_current(&mut tokens, &mut current);
            tokens.push(match character {
                '(' => Token::OpenParen,
                ')' => Token::CloseParen,
                '{' => Token::OpenBrace,
                '}' => Token::CloseBrace,
                '<' => Token::OpenAngle,
                '>' => Token::CloseAngle,
                ',' => Token::Comma,
                _ => unreachable!(),
            });
        } else {
            current.push(character);
        }
    }
    push_current(&mut tokens, &mut current);
    Ok(tokens)
}

fn push_current(tokens: &mut Vec<Token>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(Token::Atom(std::mem::take(current)));
    }
}

fn parse_sequence(
    tokens: &[Token],
    mut position: usize,
    nested: bool,
) -> Result<(Expression, usize)> {
    let mut terms = Vec::new();
    while position < tokens.len()
        && !matches!(tokens[position], Token::CloseParen | Token::CloseBrace)
    {
        let (term, next) = parse_term(tokens, position)?;
        terms.push(term);
        position = next;
    }
    if terms.is_empty() {
        let message = if nested {
            "empty parentheses"
        } else {
            "empty expression"
        };
        return Err(Error::Parse(message.into()));
    }
    if nested {
        if position >= tokens.len() {
            return Err(Error::Parse("unclosed group".into()));
        }
        let closing = &tokens[position];
        if !matches!(closing, Token::CloseParen | Token::CloseBrace) {
            return Err(Error::Parse("unclosed group".into()));
        }
        position += 1;
    }
    let mut terms = terms.into_iter();
    let mut expression = terms.next().expect("checked as non-empty");
    for term in terms {
        expression = Expression::Apply(Box::new(expression), Box::new(term));
    }
    Ok((expression, position))
}

fn parse_term(tokens: &[Token], position: usize) -> Result<(Expression, usize)> {
    if tokens.get(position + 1) == Some(&Token::OpenAngle) {
        let Token::Atom(role) = &tokens[position] else {
            return Err(Error::Parse("external role must be an Atom".into()));
        };
        if !role.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
        }) {
            return Err(Error::Parse("invalid external role name".into()));
        }
        let Some(Token::Atom(source)) = tokens.get(position + 2) else {
            return Err(Error::Parse("expected blob:ID or handle:ID".into()));
        };
        let reference = ExternalReference::parse(source)?;
        if tokens.get(position + 3) != Some(&Token::CloseAngle) {
            return Err(Error::Parse(
                "external payload must contain one reference".into(),
            ));
        }
        return Ok((
            Expression::External {
                role: role.clone(),
                reference,
            },
            position + 4,
        ));
    }
    if tokens[position] == Token::OpenParen {
        let (term, next) = parse_sequence(tokens, position + 1, true)?;
        return Ok((Expression::Group(Box::new(term)), next));
    }
    if matches!(tokens[position], Token::OpenBrace | Token::CloseBrace) {
        return Err(Error::Parse("unexpected brace".into()));
    }
    if position + 1 < tokens.len() && tokens[position + 1] == Token::OpenBrace {
        let Token::Atom(template) = &tokens[position] else {
            return Err(Error::Parse(
                "only an Atom can name a state Template".into(),
            ));
        };
        let (payload, next) = parse_payload(tokens, position + 2)?;
        return Ok((
            Expression::State {
                template: template.clone(),
                payload,
            },
            next,
        ));
    }
    let expression = match &tokens[position] {
        Token::StringLiteral(value) => Expression::StringLiteral(value.clone()),
        Token::Atom(token) => {
            if let Some(index) = token.strip_prefix('%') {
                let index = index
                    .parse::<usize>()
                    .map_err(|_| Error::Parse(format!("invalid payload reference: {token}")))?;
                Expression::PayloadRef(index)
            } else {
                Expression::Atom(token.clone())
            }
        }
        Token::CloseParen => return Err(Error::Parse("unexpected parenthesis".into())),
        Token::OpenParen
        | Token::OpenBrace
        | Token::CloseBrace
        | Token::OpenAngle
        | Token::CloseAngle
        | Token::Comma => {
            return Err(Error::Parse("unexpected grouping token".into()));
        }
    };
    Ok((expression, position + 1))
}

fn parse_payload(tokens: &[Token], mut position: usize) -> Result<(Vec<Expression>, usize)> {
    let mut payload = Vec::new();
    if position < tokens.len() && tokens[position] == Token::CloseBrace {
        return Ok((payload, position + 1));
    }
    loop {
        if position >= tokens.len() {
            return Err(Error::Parse("unclosed payload".into()));
        }
        let (term, next) = parse_term(tokens, position)?;
        payload.push(term);
        position = next;
        match tokens.get(position) {
            Some(Token::CloseBrace) => return Ok((payload, position + 1)),
            Some(Token::Comma) => {
                position += 1;
                if matches!(
                    tokens.get(position),
                    None | Some(Token::CloseBrace | Token::Comma)
                ) {
                    return Err(Error::Parse("expected payload Term after comma".into()));
                }
            }
            Some(token) => {
                return Err(Error::Parse(format!(
                    "expected comma between state payload Terms, found {}",
                    token.description()
                )));
            }
            None => return Err(Error::Parse("unclosed payload".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_is_left_associative() {
        let expected = Expression::Apply(
            Box::new(Expression::Apply(
                Box::new(Expression::Atom("a".into())),
                Box::new(Expression::Atom("b".into())),
            )),
            Box::new(Expression::Atom("c".into())),
        );
        assert_eq!(parse("a b c").unwrap(), expected);
    }

    #[test]
    fn state_may_have_an_empty_payload() {
        assert_eq!(
            parse("Empty {}").unwrap(),
            Expression::State {
                template: "Empty".into(),
                payload: Vec::new(),
            }
        );
    }

    #[test]
    fn state_payload_terms_require_commas() {
        assert_eq!(
            parse("Pair{left, right}").unwrap(),
            Expression::State {
                template: "Pair".into(),
                payload: vec![
                    Expression::Atom("left".into()),
                    Expression::Atom("right".into()),
                ],
            }
        );
        assert!(parse("Pair{left right}").is_err());
        assert!(parse("Pair{left,}").is_err());
        assert!(parse("Pair{, left}").is_err());
        assert!(parse("left, right").is_err());
    }

    #[test]
    fn group_application_remains_one_payload_term() {
        assert_eq!(
            parse("Pair{left, (add 1)}").unwrap().render_source(),
            "Pair{left, (add 1)}"
        );
    }

    #[test]
    fn parentheses_override_grouping() {
        let expected = Expression::Apply(
            Box::new(Expression::Atom("a".into())),
            Box::new(Expression::Group(Box::new(Expression::Apply(
                Box::new(Expression::Atom("b".into())),
                Box::new(Expression::Atom("c".into())),
            )))),
        );
        assert_eq!(parse("a (b c)").unwrap(), expected);
    }

    #[test]
    fn unclosed_parenthesis_is_rejected() {
        assert!(parse("a (b c").is_err());
    }

    #[test]
    fn input_is_an_ordinary_atom() {
        assert_eq!(parse("input").unwrap(), Expression::Atom("input".into()));
    }

    #[test]
    fn parses_a_payload_state_term() {
        let expected = Expression::State {
            template: "For".into(),
            payload: vec![Expression::Group(Box::new(Expression::Apply(
                Box::new(Expression::Atom("Add".into())),
                Box::new(Expression::Atom("9".into())),
            )))],
        };
        assert_eq!(parse("For { (Add 9) }").unwrap(), expected);
    }

    #[test]
    fn parses_zero_based_payload_references() {
        assert_eq!(parse("%0").unwrap(), Expression::PayloadRef(0));
        assert_eq!(parse("%12").unwrap(), Expression::PayloadRef(12));
    }

    #[test]
    fn rejects_invalid_payload_references() {
        assert!(parse("%-1").is_err());
        assert!(parse("%name").is_err());
    }

    #[test]
    fn parses_utf8_string_literals_and_basic_escapes() {
        assert_eq!(
            parse(r#""Sayo 日本語 \"quoted\" \\ \n \r \t""#).unwrap(),
            Expression::StringLiteral("Sayo 日本語 \"quoted\" \\ \n \r \t".into())
        );
        assert_eq!(
            parse(r#"left "two words" right"#).unwrap(),
            Expression::Apply(
                Box::new(Expression::Apply(
                    Box::new(Expression::Atom("left".into())),
                    Box::new(Expression::StringLiteral("two words".into())),
                )),
                Box::new(Expression::Atom("right".into())),
            )
        );
    }

    #[test]
    fn rejects_invalid_string_literals() {
        assert!(parse(r#""unknown\q""#).is_err());
        assert!(parse("\"direct\nnewline\"").is_err());
        assert!(parse(r#""unclosed"#).is_err());
    }
}
