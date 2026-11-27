//! The Roblox log -> (server address, port) parser, against the lines real clients write.
//!
//! Every case here is copied from a shipped log format (Windows `FLog::Output`/`FLog::Network`
//! and the macOS client, which writes the same markers). The overlay pings that address and
//! geolocates it, so a missed format is a HUD stuck on "Server info unavailable".

use creblox_vm_tests::game_log::{
    extract_server_info_from_log, inspect_player_log_text, is_player_log_name, pick_player_log,
    LogFile, LogSessionState,
};
use std::path::{Path, PathBuf};

fn log_file(name: &str, created_ms: u64, modified_ms: u64) -> LogFile {
    LogFile {
        path: PathBuf::from(name),
        created_ms,
        modified_ms,
    }
}

#[test]
fn udmux_address_line_wins_over_the_private_rcc_address() {
    let log = "\
2023-04-07T20:42:15.335Z,16.335245,12a0,6 [FLog::Output] Joining game 82a6e115-7889-3801-af19-351f06b3bb72 place 10694573182 at [10.12.2.219]:63976
2023-04-07T20:42:15.335Z,16.335245,12a0,7 [FLog::Network] serverId: 10.12.2.219|63976
2023-04-07T20:42:15.335Z,16.335245,12a0,7 [FLog::Network] UDMUX Address = 128.116.45.33, Port = 63976 | RCC Server Address = 10.12.2.219, Port = 63976
";
    let info = extract_server_info_from_log(log).expect("a UDMUX session has a server");
    assert_eq!(info.ip, "128.116.45.33");
    assert_eq!(info.port, 63976);
    assert_eq!(info.place_id, 10_694_573_182);
    assert_eq!(info.job_id, "82a6e115-7889-3801-af19-351f06b3bb72");
}

#[test]
fn connecting_to_udmux_server_reports_the_public_relay() {
    let log = "\
2023-08-14T08:41:23.677Z,13.677849,387c,6 [FLog::Output] Connecting to UDMUX server 128.116.4.33:62570, and RCC server 10.196.3.213:62570
2023-08-14T08:41:30.323Z,20.323877,264c,6 [FLog::Output] Connection accepted from 128.116.4.33|62570
";
    let info = extract_server_info_from_log(log).expect("a UDMUX session has a server");
    assert_eq!(info.ip, "128.116.4.33");
    assert_eq!(info.port, 62570);
}

#[test]
fn a_server_id_line_alone_is_enough() {
    let log = "2022-04-17T10:37:38.354Z,3.354337,0360,7 [FLog::Network] serverId: 128.116.86.85|55874\n";
    let info = extract_server_info_from_log(log).expect("serverId names the server");
    assert_eq!(info.ip, "128.116.86.85");
    assert_eq!(info.port, 55874);
    assert_eq!(info.place_id, 0, "no banner, so no place id");
}

#[test]
fn a_bracketed_accept_line_is_understood() {
    let log = "2026-01-02T03:04:05.000Z,1.000000,0001,6 [FLog::Output] Connection accepted from [128.116.50.9]:49152\n";
    let info = extract_server_info_from_log(log).expect("bracket spelling");
    assert_eq!(info.ip, "128.116.50.9");
    assert_eq!(info.port, 49152);
}

#[test]
fn udmux_address_without_spaces_or_port_is_still_parsed() {
    let log = "2026-10-19T09:00:00.000Z,9.000000,0001,7 [FLog::Network] UDMUX Address=128.116.127.33\n";
    let info = extract_server_info_from_log(log).expect("no-space spelling");
    assert_eq!(info.ip, "128.116.127.33");
    assert_eq!(info.port, 0, "the caller falls back to RakNet's default port");
}

#[test]
fn a_direct_connect_banner_carries_the_public_address() {
    let log = "\
2026-02-03T04:05:06.000Z,2.000000,0001,6 [FLog::Output] Joining game '6f1c2b40-0000-4000-8000-000000000001' place 1818 at 128.116.1.2
";
    let info = extract_server_info_from_log(log).expect("direct connect");
    assert_eq!(info.ip, "128.116.1.2");
    assert_eq!(info.port, 0);
    assert_eq!(info.place_id, 1818);
}

#[test]
fn the_newest_udmux_line_wins_after_a_rejoin() {
    let log = "\
2026-02-03T04:05:06.000Z,2.000000,0001,6 [FLog::Output] Joining game 'job' place 1818 at [10.12.2.219]:63976
2026-02-03T04:05:07.000Z,3.000000,0001,7 [FLog::Network] UDMUX Address = 128.116.45.33, Port = 63976
2026-02-03T04:20:00.000Z,4.000000,0001,7 [FLog::Network] UDMUX Address = 128.116.51.33, Port = 53640
";
    let info = extract_server_info_from_log(log).expect("the current session's relay");
    assert_eq!(info.ip, "128.116.51.33");
    assert_eq!(info.port, 53640);
}

#[test]
fn a_private_only_log_has_no_server_to_ping() {
    let log = "\
2026-02-03T04:05:06.000Z,2.000000,0001,6 [FLog::Output] Joining game 'job' place 1818 at [10.12.2.219]:63976
2026-02-03T04:05:07.000Z,3.000000,0001,7 [FLog::Network] serverId: 10.12.2.219|63976
2026-02-03T04:05:08.000Z,4.000000,0001,6 [FLog::Network] Connecting to 10.12.2.219:63976
";
    assert!(
        extract_server_info_from_log(log).is_none(),
        "a datacenter address can neither be geolocated nor pinged from home"
    );
}

#[test]
fn an_unquoted_join_banner_counts_as_a_play_session() {
    let log = "\
2026-02-03T04:05:06.000Z,2.000000,0001,6 [FLog::Output] Joining game 'job' place 920587237 at [10.12.2.219]:63976
";
    let (state, _) = inspect_player_log_text(log, log.len() as u64);
    assert_eq!(state, LogSessionState::Playing);
}

#[test]
fn windows_player_logs_carry_the_player_marker() {
    assert!(is_player_log_name(
        "0.618.0.6180600_20260203T040506Z_Player_abc12_last.log",
        false
    ));
    assert!(!is_player_log_name("0.618.0.6180600_Player_abc12.log", false));
    assert!(!is_player_log_name("studio_last.log", false));
}

#[test]
fn macos_player_logs_only_need_the_last_log_suffix() {
    assert!(is_player_log_name(
        "2026-02-03T04-05-06_Player_abc12_last.log",
        true
    ));
    assert!(is_player_log_name("2026-02-03T04-05-06_last.log", true));
}

#[test]
fn a_crash_handler_log_is_never_a_player_log() {
    for macos in [true, false] {
        assert!(!is_player_log_name(
            "2026-02-03T04-05-06_Player_CrashHandler_last.log",
            macos
        ));
    }
}

#[test]
fn the_log_created_closest_to_the_process_start_wins() {
    let files = [
        log_file("older_last.log", 1_000_000, 1_100_000),
        log_file("newer_last.log", 1_000_100, 1_000_200),
    ];
    assert_eq!(
        pick_player_log(&files, 1_000_080),
        Some(Path::new("newer_last.log"))
    );
}

#[test]
fn a_log_written_after_the_start_beats_an_earlier_one_on_a_tie() {
    let files = [
        log_file("stale_last.log", 1_000_000, 1_000_500),
        log_file("live_last.log", 1_000_000, 1_001_000),
    ];
    assert_eq!(
        pick_player_log(&files, 1_000_200),
        Some(Path::new("live_last.log"))
    );
}

#[test]
fn logs_from_before_the_process_started_belong_to_an_earlier_session() {
    let files = [
        log_file("yesterday_last.log", 900_000, 900_500),
        log_file("current_last.log", 1_000_000, 1_000_400),
    ];
    assert_eq!(
        pick_player_log(&files, 1_000_000),
        Some(Path::new("current_last.log"))
    );
}

#[test]
fn an_empty_directory_has_no_log_for_the_process() {
    assert!(pick_player_log(&[], 1_000_000).is_none());
}

#[test]
fn without_a_start_time_the_most_recently_written_log_wins() {
    let files = [
        log_file("first_last.log", 1_000_000, 1_000_100),
        log_file("second_last.log", 900_000, 1_000_900),
    ];
    assert_eq!(
        pick_player_log(&files, 0),
        Some(Path::new("second_last.log"))
    );
}
