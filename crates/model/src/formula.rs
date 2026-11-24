//! Formulas over a part's attributes: `now + uniform(2 h, 6 h)`,
//! `rework + 1`, `rework >= 2 and type == frame`.
//!
//! Deliberately small: numbers and times (with units), attribute names,
//! `now`, distributions (a random draw), `+ - * /`, comparisons, `and`,
//! `or`, `not`, and `type == <part type>`. Units are checked: a time can't
//! be added to a plain number. Plain numbers are always plain numbers here;
//! the model's default units don't apply.

use simulancer_engine::spec::{BinaryOp, Expr};

use crate::expr::{is_distribution, parse_number_distribution, parse_time_distribution};
use crate::units::{DefaultUnits, Dimension, parse_quantity};

/// What a formula gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Time,
    Number,
    /// A condition: true or false.
    Truth,
}

impl Kind {
    pub fn describe(self) -> &'static str {
        match self {
            Kind::Time => "a time",
            Kind::Number => "a plain number",
            Kind::Truth => "a condition",
        }
    }
}

/// The names a formula can use.
pub struct Names<'a> {
    /// Attributes, in [`simulancer_engine::Spec`] order, with their kind.
    pub attributes: &'a [(&'a str, Kind)],
    /// Part types, for `type == <name>`.
    pub entities: &'a [&'a str],
}

const TIME_UNITS: &[&str] = &["ms", "s", "min", "h", "d"];
const LENGTH_UNITS: &[&str] = &["mm", "cm", "m", "km"];
const WORDS: &[&str] = &["now", "type", "and", "or", "not"];

/// Why `name` can't be an attribute's name, if it can't: formulas would
/// read it as something else.
pub fn reserved(name: &str) -> Option<String> {
    if name.contains('-') {
        return Some(format!(
            "`{name}` can't be an attribute's name: `-` means minus in formulas (use `_`)"
        ));
    }
    let taken = WORDS.contains(&name)
        || TIME_UNITS.contains(&name)
        || LENGTH_UNITS.contains(&name)
        || is_distribution(name);
    taken.then(|| format!("`{name}` can't be an attribute's name: formulas use that word"))
}

/// Parses a formula and tells what it gives.
pub fn parse(text: &str, names: &Names) -> Result<(Expr, Kind), String> {
    let mut p = Parser {
        text,
        pos: 0,
        names,
    };
    let result = p.or()?;
    p.skip_space();
    if p.pos < text.len() {
        return Err(format!(
            "unexpected `{}` in `{text}`",
            text[p.pos..].trim_end()
        ));
    }
    Ok(result)
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    names: &'a Names<'a>,
}

impl Parser<'_> {
    fn rest(&self) -> &str {
        &self.text[self.pos..]
    }

    fn skip_space(&mut self) {
        let rest = self.rest();
        self.pos += rest.len() - rest.trim_start().len();
    }

    /// Consumes `symbol` if it comes next.
    fn eat(&mut self, symbol: &str) -> bool {
        self.skip_space();
        if self.rest().starts_with(symbol) {
            self.pos += symbol.len();
            true
        } else {
            false
        }
    }

    /// The word (letters, digits, `_`) at the cursor, without consuming it.
    fn peek_word(&mut self) -> &str {
        self.skip_space();
        let rest = self.rest();
        let starts = rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
        let end = if starts {
            rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len())
        } else {
            0
        };
        &rest[..end]
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if self.peek_word() == word {
            self.pos += word.len();
            true
        } else {
            false
        }
    }

    fn whole(&self) -> &str {
        self.text.trim()
    }

    fn or(&mut self) -> Result<(Expr, Kind), String> {
        let mut left = self.and()?;
        while self.eat_word("or") {
            let right = self.and()?;
            left = self.logic(BinaryOp::Or, "or", left, right)?;
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<(Expr, Kind), String> {
        let mut left = self.not()?;
        while self.eat_word("and") {
            let right = self.not()?;
            left = self.logic(BinaryOp::And, "and", left, right)?;
        }
        Ok(left)
    }

    fn logic(
        &self,
        op: BinaryOp,
        word: &str,
        (a, ka): (Expr, Kind),
        (b, kb): (Expr, Kind),
    ) -> Result<(Expr, Kind), String> {
        for k in [ka, kb] {
            if k != Kind::Truth {
                return Err(format!(
                    "`{word}` joins conditions, but one side is {} in `{}`",
                    k.describe(),
                    self.whole()
                ));
            }
        }
        Ok((Expr::Binary(op, Box::new(a), Box::new(b)), Kind::Truth))
    }

    fn not(&mut self) -> Result<(Expr, Kind), String> {
        if self.eat_word("not") {
            let (x, k) = self.not()?;
            if k != Kind::Truth {
                return Err(format!(
                    "`not` takes a condition, not {} in `{}`",
                    k.describe(),
                    self.whole()
                ));
            }
            return Ok((Expr::Not(Box::new(x)), Kind::Truth));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<(Expr, Kind), String> {
        if self.eat_word("type") {
            return self.type_test();
        }
        let (a, ka) = self.sum()?;
        const OPS: &[(&str, BinaryOp)] = &[
            ("<=", BinaryOp::Le),
            (">=", BinaryOp::Ge),
            ("==", BinaryOp::Eq),
            ("!=", BinaryOp::Ne),
            ("<", BinaryOp::Lt),
            (">", BinaryOp::Gt),
        ];
        let Some(&(symbol, op)) = OPS.iter().find(|(s, _)| self.eat(s)) else {
            return Ok((a, ka));
        };
        let (b, kb) = self.sum()?;
        if ka != kb {
            return Err(format!(
                "`{symbol}` compares {} with {} in `{}`",
                ka.describe(),
                kb.describe(),
                self.whole()
            ));
        }
        Ok((Expr::Binary(op, Box::new(a), Box::new(b)), Kind::Truth))
    }

    /// After `type`: `== <part type>` or `!= <part type>`.
    fn type_test(&mut self) -> Result<(Expr, Kind), String> {
        let equal = if self.eat("==") {
            true
        } else if self.eat("!=") {
            false
        } else {
            return Err(format!(
                "write `type == <part type>` or `type != <part type>` in `{}`",
                self.whole()
            ));
        };
        self.skip_space();
        let rest = self.rest();
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(rest.len());
        let name = &rest[..end];
        let Some(entity) = self.names.entities.iter().position(|e| *e == name) else {
            return Err(format!(
                "`{name}` isn't a part type (part types: {})",
                self.names.entities.join(", ")
            ));
        };
        self.pos += end;
        let test = Expr::IsType(entity);
        let test = if equal {
            test
        } else {
            Expr::Not(Box::new(test))
        };
        Ok((test, Kind::Truth))
    }

    fn sum(&mut self) -> Result<(Expr, Kind), String> {
        let mut left = self.product()?;
        loop {
            let (op, verb) = if self.eat("+") {
                (BinaryOp::Add, "add")
            } else if self.eat("-") {
                (BinaryOp::Sub, "subtract")
            } else {
                return Ok(left);
            };
            let right = self.product()?;
            let (ka, kb) = (left.1, right.1);
            if ka != kb || ka == Kind::Truth {
                return Err(format!(
                    "can't {verb} {} and {} in `{}`",
                    ka.describe(),
                    kb.describe(),
                    self.whole()
                ));
            }
            left = (Expr::Binary(op, Box::new(left.0), Box::new(right.0)), ka);
        }
    }

    fn product(&mut self) -> Result<(Expr, Kind), String> {
        use Kind::{Number, Time};
        let mut left = self.unary()?;
        loop {
            let (op, verb) = if self.eat("*") {
                (BinaryOp::Mul, "multiply")
            } else if self.eat("/") {
                (BinaryOp::Div, "divide")
            } else {
                return Ok(left);
            };
            let right = self.unary()?;
            let kind = match (op, left.1, right.1) {
                (BinaryOp::Mul, Time, Number) | (BinaryOp::Mul, Number, Time) => Some(Time),
                (BinaryOp::Div, Time, Number) => Some(Time),
                (BinaryOp::Div, Time, Time) => Some(Number),
                (_, Number, Number) => Some(Number),
                _ => None,
            };
            let Some(kind) = kind else {
                return Err(format!(
                    "can't {verb} {} by {} in `{}`",
                    left.1.describe(),
                    right.1.describe(),
                    self.whole()
                ));
            };
            left = (Expr::Binary(op, Box::new(left.0), Box::new(right.0)), kind);
        }
    }

    fn unary(&mut self) -> Result<(Expr, Kind), String> {
        if self.eat("-") {
            let (x, k) = self.unary()?;
            if k == Kind::Truth {
                return Err(format!("can't negate a condition in `{}`", self.whole()));
            }
            return Ok((Expr::Neg(Box::new(x)), k));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<(Expr, Kind), String> {
        if self.eat("(") {
            let inner = self.or()?;
            if !self.eat(")") {
                return Err(format!("missing `)` in `{}`", self.whole()));
            }
            return Ok(inner);
        }
        self.skip_space();
        if self
            .rest()
            .starts_with(|c: char| c.is_ascii_digit() || c == '.')
        {
            return self.number();
        }
        let word = self.peek_word().to_string();
        if word.is_empty() {
            return Err(match self.rest().chars().next() {
                None => format!("`{}` ends too early", self.whole()),
                Some(c) => format!("unexpected `{c}` in `{}`", self.whole()),
            });
        }
        self.pos += word.len();
        if self.rest().trim_start().starts_with('(') && is_distribution(&word) {
            return self.draw(&word);
        }
        if word == "now" {
            return Ok((Expr::Now, Kind::Time));
        }
        if let Some(i) = self.names.attributes.iter().position(|(n, _)| *n == word) {
            return Ok((Expr::Attribute(i), self.names.attributes[i].1));
        }
        let known: Vec<String> = self
            .names
            .attributes
            .iter()
            .map(|(n, _)| format!("`{n}`"))
            .collect();
        Err(format!(
            "`{word}` in `{}` isn't an attribute (attributes: {}), `now` or a distribution",
            self.whole(),
            if known.is_empty() {
                "none".into()
            } else {
                known.join(", ")
            }
        ))
    }

    /// A number, with a time unit if one follows.
    fn number(&mut self) -> Result<(Expr, Kind), String> {
        let rest = self.rest();
        let mut end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        // An exponent: `1e3`, `2.5E-4`.
        let exp = &rest[end..];
        if let Some(after) = exp.strip_prefix(['e', 'E']) {
            let sign = usize::from(after.starts_with(['+', '-']));
            if after[sign..].starts_with(|c: char| c.is_ascii_digit()) {
                let digits = after[sign..]
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(after.len() - sign);
                end += 1 + sign + digits;
            }
        }
        let literal = rest[..end].to_string();
        let Ok(value) = literal.parse::<f64>() else {
            return Err(format!("`{literal}` isn't a number in `{}`", self.whole()));
        };
        self.pos += end;
        let save = self.pos;
        let unit = self.peek_word().to_string();
        if TIME_UNITS.contains(&unit.as_str()) {
            self.pos += unit.len();
            let seconds = parse_quantity(
                &format!("{literal} {unit}"),
                Dimension::Time,
                &DefaultUnits::default(),
            )?;
            return Ok((Expr::Seconds(seconds), Kind::Time));
        }
        if LENGTH_UNITS.contains(&unit.as_str()) {
            return Err(format!(
                "formulas use times and plain numbers, not lengths like `{literal} {unit}`"
            ));
        }
        self.pos = save;
        Ok((Expr::Value(value), Kind::Number))
    }

    /// `name(...)`: a random draw of times (if its arguments have units) or
    /// of plain numbers.
    fn draw(&mut self, name: &str) -> Result<(Expr, Kind), String> {
        self.eat("(");
        let start = self.pos;
        let close = self
            .rest()
            .find(')')
            .ok_or_else(|| format!("missing `)` after `{name}(` in `{}`", self.whole()))?;
        let inner = &self.text[start..start + close];
        self.pos = start + close + 1;
        let text = format!("{name}({inner})");
        let has_units = inner
            .split(',')
            .map(|arg| arg.split_once(':').map_or(arg, |(_, v)| v).trim())
            .any(|v| v.contains(char::is_whitespace));
        if has_units {
            let distribution = parse_time_distribution(&text, &DefaultUnits::default())?;
            Ok((
                Expr::Draw {
                    distribution,
                    time: true,
                },
                Kind::Time,
            ))
        } else {
            let distribution = parse_number_distribution(&text)?;
            Ok((
                Expr::Draw {
                    distribution,
                    time: false,
                },
                Kind::Number,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulancer_engine::spec::Distribution;

    const ATTRIBUTES: &[(&str, Kind)] = &[("due", Kind::Time), ("rework", Kind::Number)];
    const NAMES: Names = Names {
        attributes: ATTRIBUTES,
        entities: &["frame", "wheel-set"],
    };

    fn kind(text: &str) -> Kind {
        parse(text, &NAMES)
            .unwrap_or_else(|e| panic!("{text}: {e}"))
            .1
    }

    fn error(text: &str) -> String {
        parse(text, &NAMES).unwrap_err()
    }

    #[test]
    fn kinds_follow_units() {
        assert_eq!(kind("now + uniform(2 h, 6 h)"), Kind::Time);
        assert_eq!(kind("rework + 1"), Kind::Number);
        assert_eq!(kind("(due - now) / 1 h"), Kind::Number);
        assert_eq!(kind("2 * 30 min"), Kind::Time);
        assert_eq!(kind("uniform(1, 10)"), Kind::Number);
        assert_eq!(kind("-rework"), Kind::Number);
        assert_eq!(kind("1.5e3"), Kind::Number);
        assert_eq!(
            kind("rework >= 2 and not (due < now or type == wheel-set)"),
            Kind::Truth
        );
        assert_eq!(kind("type != frame"), Kind::Truth);
    }

    #[test]
    fn values_are_in_seconds() {
        let (expr, _) = parse("now + 2 h", &NAMES).unwrap();
        assert_eq!(
            expr,
            Expr::Binary(
                BinaryOp::Add,
                Box::new(Expr::Now),
                Box::new(Expr::Seconds(7200.0))
            )
        );
        let (expr, _) = parse("exponential(mean: 5)", &NAMES).unwrap();
        assert_eq!(
            expr,
            Expr::Draw {
                distribution: Distribution::Exponential { mean: 5.0 },
                time: false
            }
        );
    }

    #[test]
    fn precedence() {
        let (expr, _) = parse("1 + 2 * rework", &NAMES).unwrap();
        assert_eq!(
            expr,
            Expr::Binary(
                BinaryOp::Add,
                Box::new(Expr::Value(1.0)),
                Box::new(Expr::Binary(
                    BinaryOp::Mul,
                    Box::new(Expr::Value(2.0)),
                    Box::new(Expr::Attribute(1))
                ))
            )
        );
    }

    #[test]
    fn errors() {
        let cases = [
            ("due + 1", "can't add a time and a plain number"),
            ("rework * due * due", "can't multiply a time by a time"),
            ("rework / due", "can't divide a plain number by a time"),
            ("due < 3", "compares a time with a plain number"),
            ("rework and due", "`and` joins conditions"),
            ("not rework", "`not` takes a condition"),
            ("type == bike", "`bike` isn't a part type"),
            ("type frame", "write `type == <part type>`"),
            (
                "priority + 1",
                "`priority` in `priority + 1` isn't an attribute",
            ),
            ("3 m", "not lengths"),
            ("uniform(2 h, 1 h)", "`min` < `max`"),
            ("uniform(1 h, 2)", "has no unit"),
            ("(rework + 1", "missing `)`"),
            ("rework +", "ends too early"),
            ("rework 2", "unexpected `2`"),
        ];
        for (text, expected) in cases {
            let err = error(text);
            assert!(err.contains(expected), "{text}: {err}");
        }
    }

    #[test]
    fn reserved_names() {
        assert!(reserved("due").is_none());
        assert!(reserved("now").is_some());
        assert!(reserved("min").is_some());
        assert!(reserved("uniform").is_some());
        assert!(reserved("order-size").is_some());
    }
}
