//! Auto-Follow Host: one launch per server, never a relaunch into the current one.
//!
//! `should_follow` is the rule both follow paths use - the host's launch frame and the
//! host's Roblox presence. Keyed on the job id, which is what identifies a Roblox server.

use creblox_vm_tests::party::should_follow;

#[test]
fn a_server_is_followed_once() {
    assert!(should_follow(None, None, 7001, "job-a"));
    assert!(
        !should_follow(Some("job-a"), None, 7001, "job-a"),
        "the same job must not launch twice"
    );
    assert!(
        should_follow(Some("job-a"), None, 7001, "job-b"),
        "the host moving servers is followed"
    );
}

#[test]
fn a_server_the_member_is_already_in_is_not_followed() {
    assert!(
        !should_follow(None, Some("job-a"), 7001, "job-a"),
        "the guest already plays there; relaunching would kill the client"
    );
    assert!(
        !should_follow(Some("job-a"), Some("job-a"), 7001, "job-a"),
        "both the last follow and the current session say no"
    );
    assert!(
        should_follow(None, Some("job-a"), 7001, "job-b"),
        "a different job is still followed"
    );
    assert!(
        !should_follow(None, Some("job-a"), 7002, "job-a"),
        "the job id identifies the server; a differing place does not make it a new one"
    );
}

#[test]
fn a_launch_without_a_usable_target_is_ignored() {
    assert!(!should_follow(None, None, 0, "job-a"), "no place id");
    assert!(!should_follow(None, None, 7001, ""), "no job id");
    assert!(!should_follow(None, Some(""), 7001, ""), "empty current job");
}
