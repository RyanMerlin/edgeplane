use serde_json::Value;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OutputMode {
    Human,
    Json,
}

impl OutputMode {
    pub fn is_machine(self) -> bool {
        matches!(self, Self::Json)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Json => "json",
        }
    }
}

pub fn print_value(_mode: OutputMode, value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}
