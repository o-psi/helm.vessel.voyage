//! Reusable management-plane authorities. HTTP adapters must authenticate operator
//! requests before calling administrative APIs; storage alone is not HTTP auth.
pub mod enrollment;

pub mod enrollment_http;
