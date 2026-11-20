use enzui_shell::i18n::tr;
#[derive(Clone, Debug, PartialEq)]

pub struct Answer {
    pub value: String,
    pub detail: String,
}
fn formatted(value: f64) -> Result<String, String> {
    if !value.is_finite() {
        return Err(tr("shell-result-is-not-a-finite-number").into());
    }
    if value == 0.0 {
        return Ok("0".into());
    }
    let text = if value.abs() >= 1e15 || value.abs() < 1e-9 {
        format!("{value:.10e}")
    } else {
        format!("{value:.12}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    };
    Ok(text)
}
struct Parser<'a> {
    text: &'a [u8],
    at: usize,
    work: u16,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn expression(&mut self, minimum: u8, depth: u8) -> Result<f64, String> {
        self.work += 1;
        if self.work > 128 || depth > 16 {
            return Err(tr("shell-expression-is-too-complex").into());
        }
        self.space();
        let mut left = match self.text.get(self.at).copied() {
            Some(sign @ (b'+' | b'-')) => {
                self.at += 1;
                let value = self.expression(3, depth + 1)?;
                if sign == b'-' { -value } else { value }
            }
            Some(b'(') => {
                self.at += 1;
                let value = self.expression(0, depth + 1)?;
                self.space();
                if self.text.get(self.at) != Some(&b')') {
                    return Err(tr("shell-close-the-parenthesis").into());
                }
                self.at += 1;
                value
            }
            Some(b'0'..=b'9' | b'.') => self.number()?,
            Some(b'p') if self.text.get(self.at..self.at + 2) == Some(b"pi") => {
                self.at += 2;
                std::f64::consts::PI
            }
            Some(b'e') => {
                self.at += 1;
                std::f64::consts::E
            }
            _ => return Err(tr("shell-enter-a-number-or-expression").into()),
        };
        loop {
            self.space();
            if self.text.get(self.at) == Some(&b'%') {
                self.at += 1;
                left /= 100.0;
                continue;
            }
            let Some(operator) = self.text.get(self.at).copied() else {
                break;
            };
            let (binding, right_binding) = match operator {
                b'+' | b'-' => (1, 2),
                b'*' | b'/' => (2, 3),
                b'^' => (4, 4),
                _ => break,
            };
            if binding < minimum {
                break;
            }
            self.at += 1;
            let right = self.expression(right_binding, depth + 1)?;
            left = match operator {
                b'+' => left + right,
                b'-' => left - right,
                b'*' => left * right,
                b'/' if right == 0.0 => {
                    return Err(tr("shell-cannot-divide-by-zero").into());
                }
                b'/' => left / right,
                b'^' => left.powf(right),
                _ => unreachable!(),
            };
            if !left.is_finite() {
                return Err(tr("shell-result-is-outside-the-supported-numeric-range").into());
            }
        }
        Ok(left)
    }
    fn number(&mut self) -> Result<f64, String> {
        let start = self.at;
        while self
            .text
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            self.at += 1;
        }
        if self
            .text
            .get(self.at)
            .is_some_and(|b| *b == b'e' || *b == b'E')
        {
            self.at += 1;
            if self
                .text
                .get(self.at)
                .is_some_and(|b| *b == b'+' || *b == b'-')
            {
                self.at += 1;
            }
            while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
        }
        let value: f64 = std::str::from_utf8(&self.text[start..self.at])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| tr("shell-invalid-number"))?;
        if !value.is_finite() {
            return Err(tr("shell-number-is-outside-the-supported-range").into());
        }
        Ok(value)
    }
}
fn arithmetic(query: &str) -> Result<f64, String> {
    let mut parser = Parser {
        text: query.as_bytes(),
        at: 0,
        work: 0,
    };
    let answer = parser.expression(0, 0)?;
    parser.space();
    if parser.at != parser.text.len() {
        return Err(tr("shell-unsupported-expression-use-and-parentheses").into());
    }
    Ok(answer)
}
#[derive(Clone, Copy, PartialEq)]
enum Family {
    Length,
    Mass,
    Time,
    Data,
    Temperature,
}
struct Unit {
    family: Family,
    scale: f64,
    offset: f64,
    label: &'static str,
}
fn unit(value: &str) -> Option<Unit> {
    let (family, scale, offset, label) = match value.trim().to_ascii_lowercase().as_str() {
        "mm" | "millimeter" | "millimeters" => (Family::Length, 0.001, 0.0, "mm"),
        "cm" | "centimeter" | "centimeters" => (Family::Length, 0.01, 0.0, "cm"),
        "m" | "meter" | "meters" | "metre" | "metres" => (Family::Length, 1.0, 0.0, "m"),
        "km" | "kilometer" | "kilometers" => (Family::Length, 1000.0, 0.0, "km"),
        "in" | "inch" | "inches" => (Family::Length, 0.0254, 0.0, "in"),
        "ft" | "foot" | "feet" => (Family::Length, 0.3048, 0.0, "ft"),
        "yd" | "yard" | "yards" => (Family::Length, 0.9144, 0.0, "yd"),
        "mi" | "mile" | "miles" => (Family::Length, 1609.344, 0.0, "mi"),
        "g" | "gram" | "grams" => (Family::Mass, 0.001, 0.0, "g"),
        "kg" | "kilogram" | "kilograms" => (Family::Mass, 1.0, 0.0, "kg"),
        "mg" => (Family::Mass, 0.000001, 0.0, "mg"),
        "lb" | "lbs" | "pound" | "pounds" => (Family::Mass, 0.45359237, 0.0, "lb"),
        "oz" | "ounce" | "ounces" => (Family::Mass, 0.028349523125, 0.0, "oz"),
        "s" | "sec" | "second" | "seconds" => (Family::Time, 1.0, 0.0, "s"),
        "ms" | "millisecond" | "milliseconds" => (Family::Time, 0.001, 0.0, "ms"),
        "min" | "minute" | "minutes" => (Family::Time, 60.0, 0.0, "min"),
        "h" | "hr" | "hour" | "hours" => (Family::Time, 3600.0, 0.0, "h"),
        "day" | "days" => (Family::Time, 86400.0, 0.0, "days"),
        "week" | "weeks" => (Family::Time, 604800.0, 0.0, "weeks"),
        "b" | "byte" | "bytes" => (Family::Data, 1.0, 0.0, "B"),
        "kb" => (Family::Data, 1000.0, 0.0, "kB"),
        "mb" => (Family::Data, 1e6, 0.0, "MB"),
        "gb" => (Family::Data, 1e9, 0.0, "GB"),
        "tb" => (Family::Data, 1e12, 0.0, "TB"),
        "kib" => (Family::Data, 1024.0, 0.0, "KiB"),
        "mib" => (Family::Data, 1048576.0, 0.0, "MiB"),
        "gib" => (Family::Data, 1073741824.0, 0.0, "GiB"),
        "tib" => (Family::Data, 1099511627776.0, 0.0, "TiB"),
        "c" | "°c" | "celsius" => (Family::Temperature, 1.0, 273.15, "°C"),
        "f" | "°f" | "fahrenheit" => (
            Family::Temperature,
            5.0 / 9.0,
            273.15 - 32.0 * 5.0 / 9.0,
            "°F",
        ),
        "k" | "kelvin" => (Family::Temperature, 1.0, 0.0, "K"),
        _ => return None,
    };
    Some(Unit {
        family,
        scale,
        offset,
        label,
    })
}
fn conversion(query: &str) -> Option<Result<Answer, String>> {
    let words: Vec<_> = query.split_whitespace().collect();
    if words.len() != 4 || !["to", "in"].contains(&words[2]) {
        return None;
    }
    let source = unit(words[1])?;
    let target = unit(words[3])?;
    Some((|| {
        if source.family != target.family {
            return Err(tr("shell-choose-units-of-the-same-kind").into());
        }
        let value = arithmetic(words[0])?;
        let base = value * source.scale + source.offset;
        if source.family == Family::Temperature && base < -1e-10 {
            return Err(tr("shell-temperature-is-below-absolute-zero").into());
        }
        let converted = (base - target.offset) / target.scale;
        let value = formatted(
            if source.family == Family::Temperature && converted.abs() < 1e-10 {
                0.0
            } else {
                converted
            },
        )?;
        Ok(Answer {
            value: format!("{value} {}", target.label),
            detail: format!("{} {} → {}", words[0], source.label, target.label),
        })
    })())
}
pub fn evaluate(query: &str) -> Option<Result<Answer, String>> {
    if query.len() > 1024 {
        return Some(Err(tr("shell-expression-is-too-long").into()));
    }
    let explicit = query.trim_start().starts_with('=');
    let query = query
        .trim()
        .trim_start_matches('=')
        .trim()
        .replace('×', "*")
        .replace('÷', "/")
        .replace('−', "-");
    if let Some(answer) = conversion(&query.to_lowercase()) {
        return Some(answer);
    }
    let query = query.replace("% of ", "% * ");
    let implicit = query.bytes().any(|b| b.is_ascii_digit()) || query == "pi" || query == "e";
    if !explicit
        && (!implicit
            || !query
                .bytes()
                .all(|b| b.is_ascii_digit() || b".+-*/^%() pieE\t".contains(&b)))
    {
        return None;
    }
    Some(arithmetic(&query).and_then(formatted).map(|value| Answer {
        value,
        detail: String::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn value(q: &str) -> String {
        evaluate(q).unwrap().unwrap().value
    }
    #[test]
    fn calculator_behaviour() {
        // Arithmetic obeys precedence and is finite.
        {
            assert_eq!(value("2+3*4"), "14");
            assert_eq!(value("3 - 2"), "1");
            assert_eq!(value("(2+3)*4"), "20");
            assert_eq!(value("-2^2"), "-4");
            assert_eq!(value("2^3^2"), "512");
            assert_eq!(value("15% of 240"), "36");
            assert_eq!(value("2.5e2 / 10"), "25");
            for q in [
                "1/0",
                "2+",
                "(4+3",
                "=rm -rf /",
                "1e999",
                "(-1)^0.5",
                "2^9999",
            ] {
                assert!(evaluate(q).unwrap().is_err(), "{q}");
            }
            assert!(evaluate("Text editor").is_none());
            assert!(evaluate("2026-report.txt").is_none());
        }
        // Conversions distinguish families and binary data units.
        {
            assert_eq!(value("1 km to m"), "1000 m");
            assert_eq!(value("32 F to C"), "0 °C");
            assert_eq!(value("100 C to F"), "212 °F");
            assert_eq!(value("1 GiB to MiB"), "1024 MiB");
            assert_eq!(value("1 GB to MB"), "1000 MB");
            assert_eq!(value("2 hours in min"), "120 min");
            assert!(evaluate("2 kg to m").unwrap().is_err());
            assert!(evaluate("-300 C to K").unwrap().is_err());
        }
        // Bounded work rejects deep or oversized input.
        {
            assert!(
                evaluate(&("(".repeat(20) + "1" + &")".repeat(20)))
                    .unwrap()
                    .is_err()
            );
            assert!(evaluate(&"1".repeat(1025)).unwrap().is_err());
        }
    }
}
