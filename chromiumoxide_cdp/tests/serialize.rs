//! Generated params serialize the way Chromium expects.

use chromiumoxide_cdp::cdp::browser_protocol::dom_snapshot::CaptureSnapshotParams;
use chromiumoxide_cdp::cdp::browser_protocol::network::SetBlockedUrLsParams;
use serde_json::json;

/// A required array is sent even when empty: Chromium rejects the call
/// ("Invalid parameters") when it is missing.
#[test]
fn empty_required_arrays_are_sent() {
    assert_eq!(
        serde_json::to_value(CaptureSnapshotParams::new(vec![])).unwrap(),
        json!({ "computedStyles": [] })
    );
}

/// Optional arrays are still left out when unset.
#[test]
fn unset_optional_arrays_are_left_out() {
    assert_eq!(
        serde_json::to_value(SetBlockedUrLsParams::default()).unwrap(),
        json!({})
    );
}
