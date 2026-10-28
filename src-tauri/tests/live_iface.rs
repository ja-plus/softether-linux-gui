use softether_tray_lib::softether::AccountSummary;
use softether_tray_lib::vpn;

#[test]
fn live_detect_iface() {
    // 用本机真实账户清单：账户 tet 连着，适配器 VPN
    let accounts = vec![AccountSummary {
        name: "tet".into(),
        status: "已连接".into(),
        host: "hzvpn.tracyenergy.cn".into(),
        port: Some(12443),
        hub: "tracyhz".into(),
        nic: "VPN".into(),
    }];
    println!("detected = {:?}", vpn::detect_iface(&accounts, &[]));
    assert_eq!(vpn::detect_iface(&accounts, &[]).as_deref(), Some("vpn_vpn"));
}
