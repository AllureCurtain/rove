#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiSlashCommand {
    ModelPicker,
    ModelCurrent,
    ModelQuery(String),
    ModelReset,
    Unknown(String),
}

impl TuiSlashCommand {
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return None;
        }
        let mut parts = trimmed.split_whitespace();
        let command = parts.next().unwrap_or_default();
        if command != "/model" {
            return Some(Self::Unknown(command.to_string()));
        }
        let tail = parts.collect::<Vec<_>>().join(" ");
        Some(match tail.as_str() {
            "" => Self::ModelPicker,
            "current" => Self::ModelCurrent,
            "reset" => Self::ModelReset,
            query => Self::ModelQuery(query.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::TuiSlashCommand;

    #[test]
    fn parses_model_commands_and_never_treats_unknown_slashes_as_prompts() {
        assert_eq!(TuiSlashCommand::parse("hello"), None);
        assert_eq!(
            TuiSlashCommand::parse(" /model "),
            Some(TuiSlashCommand::ModelPicker)
        );
        assert_eq!(
            TuiSlashCommand::parse("/model current"),
            Some(TuiSlashCommand::ModelCurrent)
        );
        assert_eq!(
            TuiSlashCommand::parse("/model 模型 alpha"),
            Some(TuiSlashCommand::ModelQuery("模型 alpha".to_string()))
        );
        assert_eq!(
            TuiSlashCommand::parse("/modle"),
            Some(TuiSlashCommand::Unknown("/modle".to_string()))
        );
    }
}
