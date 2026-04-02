#[derive(Debug, PartialEq)]
pub struct HealthResponse {
    pub status: &'static str,
}

pub fn execute() -> HealthResponse {
    HealthResponse { status: "ok" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_ok() {
        assert_eq!(execute().status, "ok");
    }
}
