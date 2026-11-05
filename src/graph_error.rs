use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use rand::RngExt;
use serde::Serialize;
use serde_json::{Value, json};

/// A parameter count mismatch, kept structured so the inbox can explain it without parsing Meta's text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Mismatch {
    pub location: String,
    pub sent: usize,
    pub expected: usize,
}

#[derive(Clone, Debug)]
pub struct GraphErrorData {
    pub status: StatusCode,
    pub code: i64,
    pub message: String,
    pub subcode: Option<i64>,
    pub details: Option<String>,
    pub kind: &'static str,
    pub mismatch: Option<Mismatch>,
}

/// A Graph API error shaped like Meta's. The data is boxed so `Result<_, GraphError>` stays small.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{}", .0.message)]
pub struct GraphError(Box<GraphErrorData>);

impl std::ops::Deref for GraphError {
    type Target = GraphErrorData;

    fn deref(&self) -> &GraphErrorData {
        &self.0
    }
}

impl GraphError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self(Box::new(GraphErrorData {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
            subcode: None,
            details: None,
            kind: "OAuthException",
            mismatch: None,
        }))
    }

    pub fn invalid(details: impl Into<String>) -> Self {
        Self::new(131_009, "(#131009) Parameter value is not valid").details(details)
    }

    pub fn invalid_parameter(details: impl Into<String>) -> Self {
        Self::new(100, "(#100) Invalid parameter").details(details)
    }

    pub fn required(field: &str) -> Self {
        Self::new(100, format!("(#100) The parameter {field} is required."))
    }

    pub fn count_mismatch(location: &str, sent: usize, expected: usize) -> Self {
        Self::new(132_000, "(#132000) Number of parameters does not match the expected number of params")
            .details(format!(
                "{location}: number of localizable_params ({sent}) does not match the expected number of params ({expected})"
            ))
            .with_mismatch(Mismatch { location: location.to_owned(), sent, expected })
    }

    pub fn unsupported(method: &str, id: &str) -> Self {
        Self::new(
            100,
            format!(
                "Unsupported {} request. Object with ID '{id}' does not exist, cannot be loaded due to missing permissions, or does not support this operation. Please read the Graph API documentation at https://developers.facebook.com/docs/graph-api",
                method.to_lowercase()
            ),
        )
        .subcode(33)
        .kind("GraphMethodException")
    }

    pub fn unknown() -> Self {
        Self::new(1, "An unknown error has occurred.").status(StatusCode::INTERNAL_SERVER_ERROR)
    }

    #[must_use]
    pub fn details(mut self, details: impl Into<String>) -> Self {
        self.0.details = Some(details.into());
        self
    }

    #[must_use]
    pub fn status(mut self, status: StatusCode) -> Self {
        self.0.status = status;
        self
    }

    #[must_use]
    pub fn subcode(mut self, subcode: i64) -> Self {
        self.0.subcode = Some(subcode);
        self
    }

    #[must_use]
    pub fn kind(mut self, kind: &'static str) -> Self {
        self.0.kind = kind;
        self
    }

    #[must_use]
    fn with_mismatch(mut self, mismatch: Mismatch) -> Self {
        self.0.mismatch = Some(mismatch);
        self
    }

    pub fn body(&self) -> Value {
        let mut error = json!({ "message": self.message, "type": self.kind, "code": self.code });

        if let Some(subcode) = self.subcode {
            error["error_subcode"] = json!(subcode);
        }

        if let Some(details) = &self.details {
            error["error_data"] = json!({ "messaging_product": "whatsapp", "details": details });
        }

        error["fbtrace_id"] = json!(fbtrace_id());

        json!({ "error": error })
    }

    pub fn record(&self) -> Value {
        let mut record = json!({ "code": self.code, "message": self.message });

        if let Some(details) = &self.details {
            record["details"] = json!(details);
        }

        if let Some(mismatch) = &self.mismatch {
            record["mismatch"] = json!(mismatch);
        }

        record
    }
}

impl IntoResponse for GraphError {
    fn into_response(self) -> Response {
        (self.status, axum::Json(self.body())).into_response()
    }
}

fn fbtrace_id() -> String {
    let bytes: [u8; 9] = rand::rng().random();

    base64::engine::general_purpose::STANDARD
        .encode(bytes)
        .replace(['+', '/', '='], "A")
}

impl From<crate::config::SaveError> for GraphError {
    fn from(error: crate::config::SaveError) -> Self {
        Self::unknown().details(error.0)
    }
}
