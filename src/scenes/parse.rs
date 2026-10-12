//! Reading scenes and values from text, see [`Scene::parse`] and [`parse_value`].
//!
//! ```text
//! scene    := (item (',' item)* ','?)?
//! item     := '@' type patch | entity
//! entity   := ('#' name)? (':' name)? (type patch)* ('[' (entity (',' entity)* ','?)? ']')?
//! patch    := '{' fields '}' | '(' values ')' | '=' value | nothing
//! value    := number | string | 'true' | 'false' | '#' name | '#' id | '()'
//!           | '[' values ']' | '(' values ')' | '{' fields '}'
//!           | type ('(' values ')' | '{' fields '}')?    (an enum variant, or a type annotated value)
//! fields   := (name ':' value (',' name ':' value)* ','?)?
//! name     := identifier | integer | string
//! type     := identifier ('::' identifier)* ('<' ... '>')?
//! ```
//!
//! Whitespace separates the patches of an entity.  `//` and `/* */` comments are ignored.

use anyhow::bail;

use crate::*;

impl Scene {
    /// Reads a scene from text, see the [module docs](crate::scenes::parse) for the format.
    ///
    /// # Errors
    ///
    /// Errors on malformed text, giving the line and column.  Types aren't checked
    /// until the scene spawns.
    pub fn parse(text: &str) -> anyhow::Result<Scene> {
        let mut parser = Parser { src: text, pos: 0 };
        parser.scene().map_err(|e| parser.located(e))
    }
}

/// Reads a single [`Value`] from text, such as `{ x: 1.0, tags: ["a"] }`.
///
/// # Errors
///
/// Errors on malformed text, giving the line and column.
pub fn parse_value(text: &str) -> anyhow::Result<Value> {
    let mut parser = Parser { src: text, pos: 0 };
    let result = parser.value().and_then(|value| {
        parser.skip_ws();
        if parser.peek().is_some() { bail!("unexpected `{}` after the value", parser.peek().unwrap()) }
        Ok(value)
    });
    result.map_err(|e| parser.located(e))
}

struct Parser<'a> {
    src: &'a str,
    pos: usize
}

impl <'a> Parser<'a> {
    /// `error` with the line and column of the current position.
    fn located(&self, error: anyhow::Error) -> anyhow::Error {
        let before = &self.src[..self.pos];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        error.context(format!("at line {line}, column {column}"))
    }

    fn rest(&self) -> &'a str { &self.src[self.pos..] }

    fn peek(&self) -> Option<char> { self.rest().chars().next() }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_ws(&mut self) {
        loop {
            let rest = self.rest();
            let trimmed = rest.trim_start();
            self.pos += rest.len() - trimmed.len();
            if trimmed.starts_with("//") {
                self.pos += trimmed.find('\n').unwrap_or(trimmed.len());
            } else if let Some(comment) = trimmed.strip_prefix("/*") {
                self.pos += comment.find("*/").map(|end| end + 4).unwrap_or(trimmed.len());
            } else {
                return;
            }
        }
    }

    /// Skips whitespace, then consumes `c` if it is next.
    fn eat(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(c) { self.pos += c.len_utf8(); true } else { false }
    }

    fn expect(&mut self, c: char) -> anyhow::Result<()> {
        if self.eat(c) { return Ok(()) }
        match self.peek() {
            Some(found) => bail!("expected `{c}`, found `{found}`"),
            None => bail!("expected `{c}`, found the end")
        }
    }

    fn ident(&mut self) -> Option<&'a str> {
        let rest = self.rest();
        rest.chars().next().filter(|c| c.is_alphabetic() || *c == '_')?;
        let len = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
        let ident = &self.src[self.pos..self.pos + len];
        self.pos += len;
        Some(ident)
    }

    /// A type path such as `my::Vec3<f32>`, without whitespace.
    fn type_path(&mut self) -> anyhow::Result<String> {
        let mut path = String::new();
        loop {
            let Some(ident) = self.ident() else { bail!("expected a type name") };
            path.push_str(ident);
            if !self.rest().starts_with("::") { break }
            self.pos += 2;
            path.push_str("::");
        }
        if self.peek() == Some('<') {
            let mut depth = 0;
            loop {
                let Some(c) = self.bump() else { bail!("unclosed `<` in type `{path}`") };
                if !c.is_whitespace() { path.push(c) }
                match c { '<' => depth += 1, '>' => { depth -= 1; if depth == 0 { break } } _ => {} }
            }
        }
        Ok(path)
    }

    /// An entity, scene or field name: an identifier, an integer or a string.
    fn name(&mut self) -> anyhow::Result<String> {
        self.skip_ws();
        match self.peek() {
            Some('"') => self.string(),
            Some(c) if c.is_ascii_digit() => {
                let len = self.rest().find(|c: char| !c.is_ascii_digit()).unwrap_or(self.rest().len());
                let name = self.rest()[..len].to_string();
                self.pos += len;
                Ok(name)
            }
            _ => match self.ident() {
                Some(ident) => Ok(ident.to_string()),
                None => bail!("expected a name")
            }
        }
    }

    fn string(&mut self) -> anyhow::Result<String> {
        self.expect('"')?;
        let mut value = String::new();
        loop {
            match self.bump() {
                None => bail!("unclosed string"),
                Some('"') => return Ok(value),
                Some('\\') => match self.bump() {
                    Some('n') => value.push('\n'),
                    Some('r') => value.push('\r'),
                    Some('t') => value.push('\t'),
                    Some('0') => value.push('\0'),
                    Some(c @ ('"' | '\\')) => value.push(c),
                    Some(c) => bail!("unknown escape `\\{c}`"),
                    None => bail!("unclosed string")
                },
                Some(c) => value.push(c)
            }
        }
    }

    fn number(&mut self) -> anyhow::Result<Value> {
        let start = self.pos;
        if matches!(self.peek(), Some('-' | '+')) { self.pos += 1 }
        if self.rest().starts_with("inf") {
            self.pos += 3;
            return Ok(Value::Float(if self.src[start..].starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY }));
        }
        let digits = |p: &mut Self| {
            let len = p.rest().find(|c: char| !(c.is_ascii_digit() || c == '_')).unwrap_or(p.rest().len());
            p.pos += len;
            len
        };
        if digits(self) == 0 { bail!("expected a number") }
        let mut float = false;
        if self.peek() == Some('.') && self.rest()[1..].starts_with(|c: char| c.is_ascii_digit()) {
            self.pos += 1;
            digits(self);
            float = true;
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('-' | '+')) { self.pos += 1 }
            if digits(self) == 0 { bail!("expected an exponent") }
            float = true;
        }
        let text = self.src[start..self.pos].replace('_', "");
        Ok(match float {
            true => Value::Float(text.parse()?),
            false => Value::Int(text.parse().map_err(|_| anyhow::anyhow!("integer `{text}` is out of range"))?)
        })
    }

    fn value(&mut self) -> anyhow::Result<Value> {
        self.skip_ws();
        match self.peek() {
            Some('"') => Ok(Value::Str(self.string()?)),
            Some('#') => {
                self.pos += 1;
                if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    let Value::Int(id) = self.number()? else { bail!("expected an entity id") };
                    Ok(Value::Entity(EntityRef::Id(EntityID::try_from(id)?)))
                } else {
                    Ok(Value::Entity(EntityRef::Named(self.name()?)))
                }
            }
            Some('[') => Ok(Value::List(self.values('[', ']')?)),
            Some('(') => {
                let items = self.values('(', ')')?;
                Ok(if items.is_empty() { Value::Unit } else { Value::Tuple(items) })
            }
            Some('{') => Ok(Value::Struct(self.fields()?)),
            Some(c) if c.is_ascii_digit() || c == '-' || c == '+' => self.number(),
            Some(c) if c.is_alphabetic() || c == '_' => {
                let path = self.type_path()?;
                match path.as_str() {
                    "true" => return Ok(Value::Bool(true)),
                    "false" => return Ok(Value::Bool(false)),
                    "inf" => return Ok(Value::Float(f64::INFINITY)),
                    "NaN" | "nan" => return Ok(Value::Float(f64::NAN)),
                    _ => {}
                }
                let variant = path.split('<').next().unwrap().rsplit("::").next().unwrap().to_string();
                Ok(Value::Enum { variant, value: Box::new(self.patch_body(false)?) })
            }
            Some(c) => bail!("expected a value, found `{c}`"),
            None => bail!("expected a value, found the end")
        }
    }

    /// What follows a type name: fields, a tuple, `= value` if `allow_eq`, or nothing.
    fn patch_body(&mut self, allow_eq: bool) -> anyhow::Result<Value> {
        self.skip_ws();
        match self.peek() {
            Some('{') => Ok(Value::Struct(self.fields()?)),
            Some('(') => Ok(Value::Tuple(self.values('(', ')')?)),
            Some('=') if allow_eq => { self.pos += 1; self.value() }
            _ => Ok(Value::Unit)
        }
    }

    /// Comma separated values between `open` and `close`.
    fn values(&mut self, open: char, close: char) -> anyhow::Result<Vec<Value>> {
        self.expect(open)?;
        let mut items = vec![];
        loop {
            if self.eat(close) { return Ok(items) }
            items.push(self.value()?);
            if !self.eat(',') { self.expect(close)?; return Ok(items) }
        }
    }

    fn fields(&mut self) -> anyhow::Result<Vec<(String, Value)>> {
        self.expect('{')?;
        let mut fields = vec![];
        loop {
            if self.eat('}') { return Ok(fields) }
            let name = self.name()?;
            self.expect(':')?;
            fields.push((name, self.value()?));
            if !self.eat(',') { self.expect('}')?; return Ok(fields) }
        }
    }

    fn scene(&mut self) -> anyhow::Result<Scene> {
        let mut scene = Scene::default();
        loop {
            self.skip_ws();
            if self.peek().is_none() { break }
            if self.eat('@') {
                let type_name = self.type_path()?;
                scene.resources.push(ResourcePatch::Reflected { type_name, value: self.patch_body(true)? });
            } else {
                scene.roots.push(self.entity()?);
            }
            if !self.eat(',') { break }
        }
        self.skip_ws();
        match self.peek() {
            None => Ok(scene),
            Some(c) => bail!("expected `,` between entities, found `{c}`")
        }
    }

    fn entity(&mut self) -> anyhow::Result<SceneEntity> {
        let mut entity = SceneEntity::default();
        if self.eat('#') { entity.name = Some(self.name()?) }
        self.skip_ws();
        if self.peek() == Some(':') && !self.rest().starts_with("::") {
            self.pos += 1;
            entity.inherits = Some(self.name()?);
        }
        loop {
            self.skip_ws();
            match self.peek() {
                None | Some(',' | ']') => break,
                Some('[') => {
                    self.pos += 1;
                    loop {
                        if self.eat(']') { break }
                        entity.children.push(self.entity()?);
                        if !self.eat(',') { self.expect(']')?; break }
                    }
                    break;
                }
                Some(c) if c.is_alphabetic() || c == '_' => {
                    let type_name = self.type_path()?;
                    entity.components.push(ScenePatch::Reflected { type_name, value: self.patch_body(true)? });
                }
                Some(c) => bail!("expected a component, `[`, `,` or `]`, found `{c}`")
            }
        }
        if entity.name.is_none() && entity.inherits.is_none() && entity.components.is_empty() && entity.children.is_empty() {
            bail!("expected an entity")
        }
        Ok(entity)
    }
}
