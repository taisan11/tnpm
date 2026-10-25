//! Small owned JSON tree; parsing and serialization are both handled by nojson.
use anyhow::Result;
use nojson::{DisplayJson, JsonFormatter, JsonValueKind, RawJson, RawJsonValue};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn object() -> Self {
        Self::Object(Vec::new())
    }

    pub fn parse(text: &str) -> Result<Self> {
        Ok(Self::from_raw(RawJson::parse(text)?.value())?)
    }

    fn from_raw(value: RawJsonValue<'_, '_>) -> std::result::Result<Self, nojson::JsonParseError> {
        Ok(match value.kind() {
            JsonValueKind::Null => Self::Null,
            JsonValueKind::Boolean => Self::Bool(value.try_into()?),
            JsonValueKind::Integer | JsonValueKind::Float => {
                Self::Number(value.as_raw_str().into())
            }
            JsonValueKind::String => Self::String(value.try_into()?),
            JsonValueKind::Array => Self::Array(
                value
                    .to_array()?
                    .map(Self::from_raw)
                    .collect::<std::result::Result<_, _>>()?,
            ),
            JsonValueKind::Object => Self::Object(
                value
                    .to_object()?
                    .map(|(key, value)| Ok((key.try_into()?, Self::from_raw(value)?)))
                    .collect::<std::result::Result<_, nojson::JsonParseError>>()?,
            ),
        })
    }

    pub fn get(&self, key: &str) -> Option<&Self> {
        self.members()
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Self> {
        match self {
            Self::Object(members) => members
                .iter_mut()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn set(&mut self, key: &str, value: Self) {
        if let Self::Object(members) = self {
            if let Some((_, old)) = members.iter_mut().find(|(name, _)| name == key) {
                *old = value;
            } else {
                members.push((key.into(), value));
            }
        }
    }

    pub fn remove(&mut self, key: &str) {
        if let Self::Object(members) = self {
            members.retain(|(name, _)| name != key);
        }
    }

    pub fn string(&self) -> Option<&str> {
        if let Self::String(value) = self {
            Some(value)
        } else {
            None
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Self::string)
    }

    pub fn members(&self) -> &[(String, Self)] {
        if let Self::Object(members) = self {
            members
        } else {
            &[]
        }
    }

    pub fn pretty(&self) -> String {
        nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.value(self)
        })
        .to_string()
    }

    pub fn display(&self) -> String {
        self.string()
            .map(str::to_owned)
            .unwrap_or_else(|| self.pretty())
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}

impl DisplayJson for Value {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> fmt::Result {
        match self {
            Self::Null => f.value(()),
            Self::Bool(value) => f.value(value),
            Self::Number(value) => f.value(RawJson::parse(value).map_err(|_| fmt::Error)?.value()),
            Self::String(value) => f.value(value),
            Self::Array(values) => f.array(|f| {
                for value in values {
                    f.element(value)?;
                }
                Ok(())
            }),
            Self::Object(members) => f.object(|f| {
                for (key, value) in members {
                    f.member(key, value)?;
                }
                Ok(())
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn parse_errors_preserve_their_type_with_context() {
        let error = Value::parse("[").context("invalid manifest").unwrap_err();
        assert_eq!(error.to_string(), "invalid manifest");
        assert!(error.downcast_ref::<nojson::JsonParseError>().is_some());
        assert!(error.chain().count() > 1);
    }

    #[test]
    fn roundtrip_preserves_numbers_order_and_unknown_fields() {
        let value =
            Value::parse(r#"{"z":9007199254740993,"a":[null,true,"日本語\n",1.20e3]}"#).unwrap();
        assert_eq!(Value::parse(&value.pretty()).unwrap(), value);
        assert!(value.pretty().starts_with("{\n  \"z\""));
    }
}
