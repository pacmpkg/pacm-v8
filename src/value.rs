use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsValue {
    raw: String,
}

impl JsValue {
    pub(crate) fn new(raw: String) -> Self {
        Self { raw }
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn into_string(self) -> String {
        self.raw
    }
}

impl fmt::Display for JsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

impl From<JsValue> for String {
    fn from(value: JsValue) -> Self {
        value.raw
    }
}
