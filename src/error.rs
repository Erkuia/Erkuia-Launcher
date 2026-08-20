use std::fmt;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Config,
    Login,
    Download,
    Verify,
    Mod,
    Java,
    Launch,
    Update,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "CONFIG",
            Self::Login => "LOGIN",
            Self::Download => "DOWNLOAD",
            Self::Verify => "VERIFY",
            Self::Mod => "MOD",
            Self::Java => "JAVA",
            Self::Launch => "LAUNCH",
            Self::Update => "UPDATE",
        }
    }

    pub fn default_message(self) -> &'static str {
        match self {
            Self::Config => "설정을 불러오지 못했어요.",
            Self::Login => "로그인하지 못했어요.",
            Self::Download => "파일을 내려받지 못했어요.",
            Self::Verify => "파일 검증에 실패했어요.",
            Self::Mod => "모드를 적용하지 못했어요.",
            Self::Java => "Java 21 런타임을 준비하지 못했어요.",
            Self::Launch => "Minecraft를 실행하지 못했어요.",
            Self::Update => "런처를 업데이트하지 못했어요.",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserError {
    pub code: ErrorCode,
    pub message: String,
}

impl UserError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let message = message.into();

        Self {
            code,
            message: if message.trim().is_empty() {
                code.default_message().to_string()
            } else {
                message
            },
        }
    }

    pub fn from_error(code: ErrorCode, error: &anyhow::Error) -> Self {
        Self::new(code, error.to_string())
    }
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_outermost_context() {
        let error = anyhow::anyhow!("connection reset")
            .context("RendogClient-Delta.jar 다운로드에 실패했어요.");
        let user_error = UserError::from_error(ErrorCode::Download, &error);

        assert_eq!(user_error.code, ErrorCode::Download);
        assert_eq!(
            user_error.message,
            "RendogClient-Delta.jar 다운로드에 실패했어요."
        );
    }

    #[test]
    fn falls_back_when_the_message_is_blank() {
        let user_error = UserError::new(ErrorCode::Java, "   ");

        assert_eq!(user_error.message, "Java 21 런타임을 준비하지 못했어요.");
    }

    #[test]
    fn codes_are_unique() {
        let codes = [
            ErrorCode::Config,
            ErrorCode::Login,
            ErrorCode::Download,
            ErrorCode::Verify,
            ErrorCode::Mod,
            ErrorCode::Java,
            ErrorCode::Launch,
            ErrorCode::Update,
        ];
        let mut seen: Vec<&str> = codes.iter().map(|code| code.as_str()).collect();
        seen.sort_unstable();
        let total = seen.len();
        seen.dedup();

        assert_eq!(seen.len(), total);
    }
}
