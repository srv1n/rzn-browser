//! Element reference types for RZN Browser Native
//!
//! Provides stable element targeting using EncodedId format and Input Synthesis Ladder
//! tracking. This is the Rust equivalent of extension/src/types/targets.ts

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Encoded element identifier: "frameOrdinal:backendNodeId"
/// Provides stable reference that survives DOM changes
pub type EncodedId = String;

/// Input synthesis rungs in escalation order
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InputRung {
    Dom = 1,      // Native DOM events (same-origin only)
    Scripted = 2, // Scripted MouseEvent/KeyboardEvent
    Cdp = 3,      // Chrome DevTools Protocol (works everywhere)
}

impl InputRung {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Dom),
            2 => Some(Self::Scripted),
            3 => Some(Self::Cdp),
            _ => None,
        }
    }

    /// Whether this rung can work cross-origin
    pub fn supports_cross_origin(self) -> bool {
        match self {
            Self::Dom => false,      // Same-origin only
            Self::Scripted => false, // Same-origin only
            Self::Cdp => true,       // Works everywhere
        }
    }
}

/// Element bounding box
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ElementBounds {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn is_visible(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}

/// Target specification - multiple ways to identify an element
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetSpec {
    /// Stable element identifier (preferred)
    pub encoded_id: Option<EncodedId>,

    /// CSS selector
    pub css: Option<String>,

    /// XPath expression
    pub xpath: Option<String>,

    /// Accessibility role name
    pub role_name: Option<String>,

    /// Text content or nearby text
    pub text_near: Option<String>,

    /// Optional frame context
    pub frame_ordinal: Option<u32>,
}

impl TargetSpec {
    /// Create a new TargetSpec with encoded_id
    pub fn from_encoded_id(encoded_id: impl Into<EncodedId>) -> Self {
        Self {
            encoded_id: Some(encoded_id.into()),
            css: None,
            xpath: None,
            role_name: None,
            text_near: None,
            frame_ordinal: None,
        }
    }

    /// Create a new TargetSpec with CSS selector
    pub fn from_css(css: impl Into<String>) -> Self {
        Self {
            encoded_id: None,
            css: Some(css.into()),
            xpath: None,
            role_name: None,
            text_near: None,
            frame_ordinal: None,
        }
    }

    /// Create a new TargetSpec with XPath
    pub fn from_xpath(xpath: impl Into<String>) -> Self {
        Self {
            encoded_id: None,
            css: None,
            xpath: Some(xpath.into()),
            role_name: None,
            text_near: None,
            frame_ordinal: None,
        }
    }

    /// Create a new TargetSpec with text near
    pub fn from_text_near(text_near: impl Into<String>) -> Self {
        Self {
            encoded_id: None,
            css: None,
            xpath: None,
            role_name: None,
            text_near: Some(text_near.into()),
            frame_ordinal: None,
        }
    }

    /// Set frame ordinal for cross-origin targeting
    pub fn with_frame(mut self, frame_ordinal: u32) -> Self {
        self.frame_ordinal = Some(frame_ordinal);
        self
    }

    /// Check if at least one targeting method is provided
    pub fn is_valid(&self) -> bool {
        self.encoded_id.is_some()
            || self.css.is_some()
            || self.xpath.is_some()
            || self.role_name.is_some()
            || self.text_near.is_some()
    }

    /// Check if this target requires cross-origin handling
    pub fn requires_cross_origin_handling(&self, current_frame_ordinal: u32) -> bool {
        self.frame_ordinal
            .map_or(false, |frame| frame != current_frame_ordinal)
    }

    /// Get the most stable targeting method available
    pub fn get_primary_method(&self) -> Option<&str> {
        if self.encoded_id.is_some() {
            Some("encoded_id")
        } else if self.css.is_some() {
            Some("css")
        } else if self.xpath.is_some() {
            Some("xpath")
        } else if self.role_name.is_some() {
            Some("role_name")
        } else if self.text_near.is_some() {
            Some("text_near")
        } else {
            None
        }
    }
}

/// Resolved element with stable identifier
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedElement {
    /// Stable encoded identifier
    pub encoded_id: EncodedId,

    /// Frame ordinal where element exists
    pub frame_ordinal: u32,

    /// Backend node ID from CDP
    pub backend_node_id: u64,

    /// Element's bounding box
    pub bounds: ElementBounds,

    /// Whether element is cross-origin
    pub is_cross_origin: bool,

    /// Original target spec used to find this element
    pub target_spec: TargetSpec,

    /// Cache timestamp (Unix milliseconds)
    pub resolved_at: u64,
}

impl ResolvedElement {
    pub fn new(
        frame_ordinal: u32,
        backend_node_id: u64,
        bounds: ElementBounds,
        is_cross_origin: bool,
        target_spec: TargetSpec,
    ) -> Self {
        let encoded_id = create_encoded_id(frame_ordinal, backend_node_id);
        let resolved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        Self {
            encoded_id,
            frame_ordinal,
            backend_node_id,
            bounds,
            is_cross_origin,
            target_spec,
            resolved_at,
        }
    }

    /// Check if this resolved element is still valid (not too old)
    pub fn is_cache_valid(&self, max_age_ms: u64) -> bool {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        current_time - self.resolved_at <= max_age_ms
    }

    /// Get the center point of the element
    pub fn center(&self) -> (f64, f64) {
        self.bounds.center()
    }

    /// Check if the element is visible
    pub fn is_visible(&self) -> bool {
        self.bounds.is_visible()
    }
}

/// Result envelope that tracks which input rung was used
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultEnvelope<T = serde_json::Value> {
    /// The actual result data
    pub result: T,

    /// Which input rung was used (1=DOM, 2=SCRIPTED, 3=CDP)
    pub rung_used: InputRung,

    /// Whether input escalated from a lower rung
    pub escalated: bool,

    /// Success/failure status
    pub success: bool,

    /// Error message if failed
    pub error: Option<String>,

    /// Performance metrics
    pub execution_time_ms: u64,

    /// Resolved element used (if applicable)
    pub resolved_element: Option<ResolvedElement>,
}

impl<T> ResultEnvelope<T> {
    pub fn success(result: T, rung_used: InputRung, execution_time_ms: u64) -> Self {
        Self {
            result,
            rung_used,
            escalated: false,
            success: true,
            error: None,
            execution_time_ms,
            resolved_element: None,
        }
    }

    pub fn success_with_element(
        result: T,
        rung_used: InputRung,
        execution_time_ms: u64,
        resolved_element: ResolvedElement,
    ) -> Self {
        Self {
            result,
            rung_used,
            escalated: false,
            success: true,
            error: None,
            execution_time_ms,
            resolved_element: Some(resolved_element),
        }
    }

    pub fn success_escalated(
        result: T,
        rung_used: InputRung,
        execution_time_ms: u64,
        resolved_element: Option<ResolvedElement>,
    ) -> Self {
        Self {
            result,
            rung_used,
            escalated: true,
            success: true,
            error: None,
            execution_time_ms,
            resolved_element,
        }
    }

    pub fn error(rung_used: InputRung, error: String, execution_time_ms: u64) -> Self
    where
        T: Default,
    {
        Self {
            result: T::default(),
            rung_used,
            escalated: false,
            success: false,
            error: Some(error),
            execution_time_ms,
            resolved_element: None,
        }
    }

    /// Convert to a different result type
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> ResultEnvelope<U> {
        ResultEnvelope {
            result: f(self.result),
            rung_used: self.rung_used,
            escalated: self.escalated,
            success: self.success,
            error: self.error,
            execution_time_ms: self.execution_time_ms,
            resolved_element: self.resolved_element,
        }
    }
}

// EncodedId helper functions

/// Validate EncodedId format: "frameOrdinal:backendNodeId"
pub fn is_valid_encoded_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split(':').collect();
    if parts.len() != 2 {
        return false;
    }

    parts[0].parse::<u32>().is_ok() && parts[1].parse::<u64>().is_ok()
}

/// Parse EncodedId into frame ordinal and backend node ID
pub fn parse_encoded_id(encoded_id: &str) -> Result<(u32, u64), String> {
    let parts: Vec<&str> = encoded_id.split(':').collect();
    if parts.len() != 2 {
        return Err("EncodedId must be in format 'frameOrdinal:backendNodeId'".to_string());
    }

    let frame_ordinal = parts[0]
        .parse::<u32>()
        .map_err(|_| "Invalid frame ordinal in EncodedId")?;
    let backend_node_id = parts[1]
        .parse::<u64>()
        .map_err(|_| "Invalid backend node ID in EncodedId")?;

    Ok((frame_ordinal, backend_node_id))
}

/// Create EncodedId from frame ordinal and backend node ID
pub fn create_encoded_id(frame_ordinal: u32, backend_node_id: u64) -> EncodedId {
    format!("{}:{}", frame_ordinal, backend_node_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoded_id_validation() {
        assert!(is_valid_encoded_id("0:123"));
        assert!(is_valid_encoded_id("1:456789"));
        assert!(!is_valid_encoded_id("invalid"));
        assert!(!is_valid_encoded_id("0"));
        assert!(!is_valid_encoded_id("0:abc"));
    }

    #[test]
    fn test_parse_encoded_id() {
        assert_eq!(parse_encoded_id("0:123").unwrap(), (0, 123));
        assert_eq!(parse_encoded_id("5:789").unwrap(), (5, 789));
        assert!(parse_encoded_id("invalid").is_err());
    }

    #[test]
    fn test_create_encoded_id() {
        assert_eq!(create_encoded_id(0, 123), "0:123");
        assert_eq!(create_encoded_id(5, 789), "5:789");
    }

    #[test]
    fn test_target_spec_validation() {
        let spec = TargetSpec::from_css("button");
        assert!(spec.is_valid());

        let empty_spec = TargetSpec {
            encoded_id: None,
            css: None,
            xpath: None,
            role_name: None,
            text_near: None,
            frame_ordinal: None,
        };
        assert!(!empty_spec.is_valid());
    }

    #[test]
    fn test_input_rung_cross_origin() {
        assert!(!InputRung::Dom.supports_cross_origin());
        assert!(!InputRung::Scripted.supports_cross_origin());
        assert!(InputRung::Cdp.supports_cross_origin());
    }

    #[test]
    fn test_result_envelope_creation() {
        let envelope = ResultEnvelope::success("test", InputRung::Dom, 100);
        assert!(envelope.success);
        assert_eq!(envelope.rung_used, InputRung::Dom);
        assert!(!envelope.escalated);

        let error_envelope: ResultEnvelope<String> =
            ResultEnvelope::error(InputRung::Cdp, "Failed to find element".to_string(), 200);
        assert!(!error_envelope.success);
        assert_eq!(error_envelope.rung_used, InputRung::Cdp);
    }

    #[test]
    fn test_element_bounds() {
        let bounds = ElementBounds::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(bounds.center(), (60.0, 45.0));
        assert!(bounds.is_visible());

        let zero_bounds = ElementBounds::new(0.0, 0.0, 0.0, 0.0);
        assert!(!zero_bounds.is_visible());
    }
}
