use std::process::Command;

#[test]
fn cli_reports_inventory_collector_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_bmc-collector"))
        .arg("--version")
        .output()
        .expect("run collector --version");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 version"),
        format!(
            "fleet-intelligence-oob {}\n",
            bmc_inventory::COLLECTOR_VERSION
        )
    );
}
