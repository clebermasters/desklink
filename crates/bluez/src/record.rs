//! The SDP service record that tells a host "this is a keyboard and mouse". BlueZ publishes it
//! for us; hosts read it while pairing and cache it, descriptor included.
use desklink_hid::descriptor::DESCRIPTOR;

/// The HID profile's service class.
pub const HID_UUID: &str = "00001124-0000-1000-8000-00805f9b34fb";
/// HID control channel.
pub const PSM_CONTROL: u16 = 0x11;
/// HID interrupt channel (input reports).
pub const PSM_INTERRUPT: u16 = 0x13;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn escape(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .flat_map(|c| match c {
            '&' => "&amp;".chars().collect::<Vec<_>>(),
            '<' => "&lt;".chars().collect(),
            '>' => "&gt;".chars().collect(),
            '"' => "&quot;".chars().collect(),
            '\'' => "&apos;".chars().collect(),
            c => vec![c],
        })
        .collect()
}

/// A combined keyboard and pointing device (subclass 0xC0) that reconnects by itself.
pub fn service_record(name: &str) -> String {
    let name = escape(name);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
<record>
  <attribute id="0x0001"><sequence><uuid value="0x1124" /></sequence></attribute>
  <attribute id="0x0004">
    <sequence>
      <sequence><uuid value="0x0100" /><uint16 value="0x{PSM_CONTROL:04x}" /></sequence>
      <sequence><uuid value="0x0011" /></sequence>
    </sequence>
  </attribute>
  <attribute id="0x0005"><sequence><uuid value="0x1002" /></sequence></attribute>
  <attribute id="0x0006">
    <sequence><uint16 value="0x656e" /><uint16 value="0x006a" /><uint16 value="0x0100" /></sequence>
  </attribute>
  <attribute id="0x0009">
    <sequence><sequence><uuid value="0x1124" /><uint16 value="0x0101" /></sequence></sequence>
  </attribute>
  <attribute id="0x000d">
    <sequence>
      <sequence>
        <sequence><uuid value="0x0100" /><uint16 value="0x{PSM_INTERRUPT:04x}" /></sequence>
        <sequence><uuid value="0x0011" /></sequence>
      </sequence>
    </sequence>
  </attribute>
  <attribute id="0x0100"><text value="{name}" /></attribute>
  <attribute id="0x0101"><text value="Keyboard and mouse" /></attribute>
  <attribute id="0x0102"><text value="Desklink" /></attribute>
  <attribute id="0x0201"><uint16 value="0x0111" /></attribute>
  <attribute id="0x0202"><uint8 value="0xc0" /></attribute>
  <attribute id="0x0203"><uint8 value="0x00" /></attribute>
  <attribute id="0x0204"><boolean value="true" /></attribute>
  <attribute id="0x0205"><boolean value="true" /></attribute>
  <attribute id="0x0206">
    <sequence>
      <sequence><uint8 value="0x22" /><text encoding="hex" value="{descriptor}" /></sequence>
    </sequence>
  </attribute>
  <attribute id="0x0207">
    <sequence><sequence><uint16 value="0x0409" /><uint16 value="0x0100" /></sequence></sequence>
  </attribute>
  <attribute id="0x0209"><boolean value="false" /></attribute>
  <attribute id="0x020a"><boolean value="true" /></attribute>
  <attribute id="0x020b"><uint16 value="0x0100" /></attribute>
  <attribute id="0x020c"><uint16 value="0x0c80" /></attribute>
  <attribute id="0x020d"><boolean value="true" /></attribute>
  <attribute id="0x020e"><boolean value="false" /></attribute>
</record>
"#,
        descriptor = hex(DESCRIPTOR),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_announces_both_channels_and_the_descriptor() {
        let record = service_record("Desklink");
        assert!(
            record.contains(r#"<uint16 value="0x0011" />"#),
            "control PSM"
        );
        assert!(
            record.contains(r#"<uint16 value="0x0013" />"#),
            "interrupt PSM"
        );
        assert!(record.contains(&hex(DESCRIPTOR)));
        assert!(
            record.contains(r#"<uint8 value="0xc0" />"#),
            "keyboard + pointer"
        );
    }

    #[test]
    fn names_cannot_break_the_xml() {
        let record = service_record("A \"<b>\" & 'c'\n");
        assert!(record.contains("A &quot;&lt;b&gt;&quot; &amp; &apos;c&apos;"));
        assert_eq!(record.matches("<record>").count(), 1);
    }
}
