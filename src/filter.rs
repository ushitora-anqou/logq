use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum FilterValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterOp {
    Contains,
    RegexMatch,
    NotContains,
    NotRegexMatch,
    JsonEquals,
    JsonNotEquals,
    JsonRegexMatch,
    JsonNotRegexMatch,
}

#[derive(Debug, Clone)]
pub struct FilterCondition {
    pub operator: FilterOp,
    pub value: FilterValue,
    pub regex: Option<regex::Regex>,
    pub json_key: Option<String>,
}

#[derive(Debug, Clone)]
pub enum JsonExpr {
    Condition(FilterCondition),
    And(Box<JsonExpr>, Box<JsonExpr>),
    Or(Box<JsonExpr>, Box<JsonExpr>),
}

#[derive(Debug, Clone)]
pub enum FilterSegment {
    Plain(FilterCondition),
    Json(JsonExpr),
    LineFormat(LineFormatTemplate),
}

#[derive(Debug, Clone)]
enum TemplatePart {
    Literal(String),
    Key(String),
}

#[derive(Debug, Clone)]
pub struct LineFormatTemplate {
    parts: Vec<TemplatePart>,
}

impl LineFormatTemplate {
    pub fn format(&self, text: &str) -> Option<String> {
        let value: Value = serde_json::from_str(text).ok()?;
        let mut result = String::new();
        for part in &self.parts {
            match part {
                TemplatePart::Literal(s) => result.push_str(s),
                TemplatePart::Key(key) => {
                    if let Some(v) = lookup_json_key(&value, key) {
                        result.push_str(&json_value_to_string(v));
                    }
                }
            }
        }
        Some(result)
    }
}

pub fn parse_line_format_template(input: &str) -> Result<LineFormatTemplate, String> {
    let mut parts = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut pos = 0;
    let mut literal_start = 0;

    while pos < len {
        if chars[pos] == '{' && pos + 1 < len && chars[pos + 1] == '{' {
            // Flush literal before {{
            if literal_start < pos {
                let s: String = chars[literal_start..pos].iter().collect();
                parts.push(TemplatePart::Literal(s));
            }
            pos += 2;
            // Skip whitespace after {{
            while pos < len && chars[pos] == ' ' {
                pos += 1;
            }
            // Expect '.'
            if pos >= len || chars[pos] != '.' {
                return Err(t!("filter.error.expected_dot_after_brace").to_string());
            }
            pos += 1;
            let key_start = pos;
            while pos < len
                && (chars[pos].is_alphanumeric() || chars[pos] == '.' || chars[pos] == '_')
            {
                pos += 1;
            }
            let key: String = chars[key_start..pos].iter().collect();
            if key.is_empty() {
                return Err(t!("filter.error.empty_key").to_string());
            }
            // Skip whitespace before }}
            while pos < len && chars[pos] == ' ' {
                pos += 1;
            }
            if pos + 1 >= len || chars[pos] != '}' || chars[pos + 1] != '}' {
                return Err(t!("filter.error.unterminated_brace").to_string());
            }
            pos += 2;
            parts.push(TemplatePart::Key(key));
            literal_start = pos;
        } else {
            pos += 1;
        }
    }

    if literal_start < len {
        let s: String = chars[literal_start..len].iter().collect();
        parts.push(TemplatePart::Literal(s));
    }

    Ok(LineFormatTemplate { parts })
}

#[derive(Debug, Clone)]
pub struct FilterQuery {
    pub segments: Vec<FilterSegment>,
}

impl FilterValue {
    fn display_string(&self) -> String {
        match self {
            FilterValue::String(s) => format!("\"{}\"", s),
            FilterValue::Number(n) => {
                if n.fract() == 0.0 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            FilterValue::Boolean(b) => b.to_string(),
            FilterValue::Null => "null".to_string(),
        }
    }
}

impl FilterQuery {
    pub fn display_string(&self) -> String {
        self.segments
            .iter()
            .map(|seg| match seg {
                FilterSegment::Plain(c) => {
                    let op = match c.operator {
                        FilterOp::Contains => "|=",
                        FilterOp::RegexMatch => "|~",
                        FilterOp::NotContains => "!=",
                        FilterOp::NotRegexMatch => "!~",
                        _ => unreachable!(),
                    };
                    format!("{} {}", op, c.value.display_string())
                }
                FilterSegment::Json(expr) => {
                    format!("| {}", expr.display_string_inner(false))
                }
                FilterSegment::LineFormat(t) => {
                    let template_str: String = t
                        .parts
                        .iter()
                        .map(|p| match p {
                            TemplatePart::Literal(s) => s.clone(),
                            TemplatePart::Key(k) => format!("{{{{ .{} }}}}", k),
                        })
                        .collect();
                    format!("| line_format \"{}\"", template_str)
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl JsonExpr {
    fn display_string_inner(&self, parent_is_and: bool) -> String {
        match self {
            JsonExpr::Condition(c) => {
                let op = match c.operator {
                    FilterOp::JsonEquals => "=",
                    FilterOp::JsonNotEquals => "!=",
                    FilterOp::JsonRegexMatch => "=~",
                    FilterOp::JsonNotRegexMatch => "!~",
                    _ => unreachable!(),
                };
                format!(
                    "{} {} {}",
                    c.json_key.as_deref().unwrap(),
                    op,
                    c.value.display_string()
                )
            }
            JsonExpr::And(l, r) => {
                format!(
                    "{} and {}",
                    l.display_string_inner(true),
                    r.display_string_inner(true)
                )
            }
            JsonExpr::Or(l, r) => {
                let inner = format!(
                    "{} or {}",
                    l.display_string_inner(false),
                    r.display_string_inner(false)
                );
                if parent_is_and {
                    format!("({})", inner)
                } else {
                    inner
                }
            }
        }
    }
}

pub fn lookup_json_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

pub fn compare_json_value(actual: &Value, expected: &FilterValue) -> bool {
    match (actual, expected) {
        (Value::String(s), FilterValue::String(e)) => s == e,
        (Value::Number(n), FilterValue::Number(e)) => n.as_f64() == Some(*e),
        (Value::Bool(b), FilterValue::Boolean(e)) => b == e,
        (Value::Null, FilterValue::Null) => true,
        _ => false,
    }
}

pub fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

fn skip_whitespace(chars: &[char], pos: &mut usize, len: usize) {
    while *pos < len && chars[*pos] == ' ' {
        *pos += 1;
    }
}

fn is_keyword(chars: &[char], pos: usize, len: usize, keyword: &str) -> bool {
    let kw_chars: Vec<char> = keyword.chars().collect();
    let kw_len = kw_chars.len();
    if pos + kw_len > len {
        return false;
    }
    if chars[pos..pos + kw_len] != kw_chars[..] {
        return false;
    }
    // Word boundary: followed by space, '(', ')', or end of input
    if pos + kw_len < len {
        let next = chars[pos + kw_len];
        next == ' ' || next == '(' || next == ')'
    } else {
        true
    }
}

fn parse_json_condition(
    chars: &[char],
    pos: &mut usize,
    len: usize,
) -> Result<FilterCondition, String> {
    // Read key name
    let key_start = *pos;
    while *pos < len
        && (chars[*pos].is_alphanumeric()
            || chars[*pos] == '_'
            || chars[*pos] == '-'
            || chars[*pos] == '.')
    {
        *pos += 1;
    }
    let key: String = chars[key_start..*pos].iter().collect();
    if key.is_empty() {
        return Err(t!("filter.error.expected_key_name", pos = pos).to_string());
    }

    skip_whitespace(chars, pos, len);

    // Read operator: =, !=, =~, !~
    let (op, needs_regex) = if *pos + 1 < len && chars[*pos] == '!' {
        if chars[*pos + 1] == '=' {
            *pos += 2;
            (FilterOp::JsonNotEquals, false)
        } else if chars[*pos + 1] == '~' {
            *pos += 2;
            (FilterOp::JsonNotRegexMatch, true)
        } else {
            return Err(t!("filter.error.expected_json_operator", pos = pos).to_string());
        }
    } else if *pos < len && chars[*pos] == '=' {
        *pos += 1;
        if *pos < len && chars[*pos] == '~' {
            *pos += 1;
            (FilterOp::JsonRegexMatch, true)
        } else {
            (FilterOp::JsonEquals, false)
        }
    } else {
        return Err(t!("filter.error.expected_json_operator", pos = pos).to_string());
    };

    skip_whitespace(chars, pos, len);

    // Read value
    let (value, regex) = if *pos < len && chars[*pos] == '"' {
        let s = parse_quoted_string(chars, pos, len)?;
        let r =
            if needs_regex {
                Some(regex::Regex::new(&s).map_err(|e| {
                    t!("filter.error.invalid_regex", err = e.to_string()).to_string()
                })?)
            } else {
                None
            };
        (FilterValue::String(s), r)
    } else if *pos + 3 < len && chars[*pos..*pos + 4] == ['t', 'r', 'u', 'e'] {
        *pos += 4;
        (FilterValue::Boolean(true), None)
    } else if *pos + 4 < len && chars[*pos..*pos + 5] == ['f', 'a', 'l', 's', 'e'] {
        *pos += 5;
        (FilterValue::Boolean(false), None)
    } else if *pos + 3 < len && chars[*pos..*pos + 4] == ['n', 'u', 'l', 'l'] {
        *pos += 4;
        (FilterValue::Null, None)
    } else {
        // Number
        let num_start = *pos;
        if *pos < len && (chars[*pos] == '-' || chars[*pos] == '+') {
            *pos += 1;
        }
        let has_digits = {
            let start = *pos;
            while *pos < len && chars[*pos].is_ascii_digit() {
                *pos += 1;
            }
            *pos > start
        };
        if !has_digits {
            return Err(t!("filter.error.expected_value", pos = num_start).to_string());
        }
        if *pos < len && chars[*pos] == '.' {
            *pos += 1;
            while *pos < len && chars[*pos].is_ascii_digit() {
                *pos += 1;
            }
        }
        let num_str: String = chars[num_start..*pos].iter().collect();
        let n: f64 = num_str
            .parse()
            .map_err(|_| t!("filter.error.invalid_number", num = num_str.as_str()).to_string())?;
        (FilterValue::Number(n), None)
    };

    Ok(FilterCondition {
        operator: op,
        value,
        regex,
        json_key: Some(key),
    })
}

fn parse_json_primary(chars: &[char], pos: &mut usize, len: usize) -> Result<JsonExpr, String> {
    skip_whitespace(chars, pos, len);

    if *pos < len && chars[*pos] == '(' {
        *pos += 1; // skip '('
        let expr = parse_json_or_expr(chars, pos, len)?;
        skip_whitespace(chars, pos, len);
        if *pos >= len || chars[*pos] != ')' {
            return Err(t!("filter.error.expected_paren").to_string());
        }
        *pos += 1; // skip ')'
        Ok(expr)
    } else {
        let cond = parse_json_condition(chars, pos, len)?;
        Ok(JsonExpr::Condition(cond))
    }
}

fn parse_json_and_expr(chars: &[char], pos: &mut usize, len: usize) -> Result<JsonExpr, String> {
    let mut left = parse_json_primary(chars, pos, len)?;

    loop {
        skip_whitespace(chars, pos, len);
        if is_keyword(chars, *pos, len, "and") {
            *pos += 3; // consume "and"
            let right = parse_json_primary(chars, pos, len)?;
            left = JsonExpr::And(Box::new(left), Box::new(right));
        } else {
            break;
        }
    }

    Ok(left)
}

fn parse_json_or_expr(chars: &[char], pos: &mut usize, len: usize) -> Result<JsonExpr, String> {
    let mut left = parse_json_and_expr(chars, pos, len)?;

    loop {
        skip_whitespace(chars, pos, len);
        if is_keyword(chars, *pos, len, "or") {
            *pos += 2; // consume "or"
            let right = parse_json_and_expr(chars, pos, len)?;
            left = JsonExpr::Or(Box::new(left), Box::new(right));
        } else {
            break;
        }
    }

    Ok(left)
}

impl FilterQuery {
    pub fn matches(&self, text: &str) -> bool {
        self.segments.iter().all(|seg| seg.matches(text))
    }
}

impl FilterSegment {
    fn matches(&self, text: &str) -> bool {
        match self {
            FilterSegment::Plain(c) => c.plain_matches(text),
            FilterSegment::Json(expr) => expr.matches(text),
            FilterSegment::LineFormat(_) => true,
        }
    }
}

impl JsonExpr {
    fn matches(&self, text: &str) -> bool {
        match self {
            JsonExpr::Condition(c) => c.json_matches(text),
            JsonExpr::And(l, r) => l.matches(text) && r.matches(text),
            JsonExpr::Or(l, r) => l.matches(text) || r.matches(text),
        }
    }
}

impl FilterCondition {
    fn plain_matches(&self, text: &str) -> bool {
        match self.operator {
            FilterOp::Contains => match &self.value {
                FilterValue::String(s) => text.contains(s.as_str()),
                _ => false,
            },
            FilterOp::NotContains => match &self.value {
                FilterValue::String(s) => !text.contains(s.as_str()),
                _ => false,
            },
            FilterOp::RegexMatch => self.regex.as_ref().unwrap().is_match(text),
            FilterOp::NotRegexMatch => !self.regex.as_ref().unwrap().is_match(text),
            _ => false,
        }
    }

    fn json_matches(&self, text: &str) -> bool {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return false;
        };
        let key = self.json_key.as_deref().unwrap();
        let target = lookup_json_key(&value, key);
        let Some(target) = target else {
            return matches!(
                self.operator,
                FilterOp::JsonNotEquals | FilterOp::JsonNotRegexMatch
            );
        };
        match self.operator {
            FilterOp::JsonEquals => compare_json_value(target, &self.value),
            FilterOp::JsonNotEquals => !compare_json_value(target, &self.value),
            FilterOp::JsonRegexMatch => {
                let s = json_value_to_string(target);
                self.regex.as_ref().unwrap().is_match(&s)
            }
            FilterOp::JsonNotRegexMatch => {
                let s = json_value_to_string(target);
                !self.regex.as_ref().unwrap().is_match(&s)
            }
            _ => false,
        }
    }
}

pub fn parse_filter_query(input: &str) -> Result<FilterQuery, String> {
    let s = input.trim();
    if s.is_empty() {
        return Ok(FilterQuery { segments: vec![] });
    }

    let mut segments: Vec<FilterSegment> = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut pos = 0;

    while pos < len {
        skip_whitespace(&chars, &mut pos, len);
        if pos >= len {
            break;
        }

        if chars[pos] == '|' {
            if pos + 1 < len && (chars[pos + 1] == '=' || chars[pos + 1] == '~') {
                // Plain text operators: |= or |~
                let op: FilterOp = match chars[pos + 1] {
                    '=' => FilterOp::Contains,
                    '~' => FilterOp::RegexMatch,
                    _ => unreachable!(),
                };
                pos += 2;

                skip_whitespace(&chars, &mut pos, len);

                let value = parse_quoted_string(&chars, &mut pos, len)?;
                let regex = match op {
                    FilterOp::RegexMatch => Some(regex::Regex::new(&value).map_err(|e| {
                        t!("filter.error.invalid_regex", err = e.to_string()).to_string()
                    })?),
                    _ => None,
                };

                segments.push(FilterSegment::Plain(FilterCondition {
                    operator: op,
                    value: FilterValue::String(value),
                    regex,
                    json_key: None,
                }));
            } else {
                // Check for | line_format or JSON key group
                pos += 1; // skip '|'
                skip_whitespace(&chars, &mut pos, len);

                if is_keyword(&chars, pos, len, "line_format") {
                    pos += "line_format".len();
                    skip_whitespace(&chars, &mut pos, len);
                    let template_str = parse_quoted_string(&chars, &mut pos, len)?;
                    let template = parse_line_format_template(&template_str)?;
                    segments.push(FilterSegment::LineFormat(template));
                } else {
                    let expr = parse_json_or_expr(&chars, &mut pos, len)?;
                    segments.push(FilterSegment::Json(expr));
                }
            }
        } else if chars[pos] == '!' {
            // Plain text operators: != or !~
            if pos + 1 >= len {
                return Err(t!("filter.error.expected_operator", pos = pos).to_string());
            }
            let op: FilterOp = match chars[pos + 1] {
                '=' => FilterOp::NotContains,
                '~' => FilterOp::NotRegexMatch,
                _ => return Err(t!("filter.error.expected_text_operator", pos = pos).to_string()),
            };
            pos += 2;

            skip_whitespace(&chars, &mut pos, len);

            let value = parse_quoted_string(&chars, &mut pos, len)?;
            let regex = match op {
                FilterOp::NotRegexMatch => Some(regex::Regex::new(&value).map_err(|e| {
                    t!("filter.error.invalid_regex", err = e.to_string()).to_string()
                })?),
                _ => None,
            };

            segments.push(FilterSegment::Plain(FilterCondition {
                operator: op,
                value: FilterValue::String(value),
                regex,
                json_key: None,
            }));
        } else {
            let value: String = chars[pos..].iter().collect();
            segments.push(FilterSegment::Plain(FilterCondition {
                operator: FilterOp::Contains,
                value: FilterValue::String(value),
                regex: None,
                json_key: None,
            }));
            break;
        }
    }

    Ok(FilterQuery { segments })
}

fn parse_quoted_string(chars: &[char], pos: &mut usize, len: usize) -> Result<String, String> {
    if *pos >= len || chars[*pos] != '"' {
        return Err(t!("filter.error.expected_quote", pos = *pos).to_string());
    }
    *pos += 1;
    let mut value = String::new();
    loop {
        if *pos >= len {
            return Err(t!("filter.error.unterminated_string").to_string());
        }
        match chars[*pos] {
            '\\' => {
                *pos += 1;
                if *pos >= len {
                    return Err(t!("filter.error.unterminated_escape").to_string());
                }
                value.push('\\');
                value.push(chars[*pos]);
                *pos += 1;
            }
            '"' => {
                *pos += 1;
                break;
            }
            _ => {
                value.push(chars[*pos]);
                *pos += 1;
            }
        }
    }
    Ok(value)
}
