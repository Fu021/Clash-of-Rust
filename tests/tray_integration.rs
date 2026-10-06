#[test]
#[ignore = "requires an interactive desktop session"]
fn native_tray_can_start_and_shutdown() {
    let guard = clash_of_rust::tray::start().expect("native tray should be created");
    drop(guard);
}
