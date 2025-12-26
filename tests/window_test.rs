use std::{thread::sleep, time::Duration};

use easy_tool::*;

#[test]
fn test_test_window() {
    test_window();
}

#[test]
fn test_time_trans_window() {
    time_trans_window();
}

#[test]
fn test_listen_keybord_event() {
    init_log().unwrap();
    init_rdev(keybord_event_handle).unwrap();
    sleep(Duration::from_secs(20));
}