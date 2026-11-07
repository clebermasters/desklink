//! Clipboard notification compatibility across portal implementations.
use anyhow::{Context, Result, bail, ensure};
use ashpd::{
    zbus,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};
use std::collections::HashMap;

pub struct Selection {
    pub session: OwnedObjectPath,
    pub mimes: Vec<String>,
    pub owner: bool,
}

pub fn parse(message: &zbus::Message) -> Result<Selection> {
    let (session, fields): (OwnedObjectPath, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .context("Invalid clipboard selection signal")?;
    let mimes = fields
        .get("mime_types")
        .map(|v| mime_types(v))
        .transpose()?
        .unwrap_or_default();
    let owner = fields
        .get("session_is_owner")
        .map(|v| bool::try_from(v).context("Invalid clipboard owner flag"))
        .transpose()?
        .unwrap_or(false);
    Ok(Selection {
        session,
        mimes,
        owner,
    })
}

fn mime_types(value: &Value<'_>) -> Result<Vec<String>> {
    // GNOME 48 wraps its string array in a one-field structure. Other portals
    // send the array directly, as specified. Accept exactly these two forms.
    let array = match value {
        Value::Array(array) => array,
        Value::Structure(s) if s.fields().len() == 1 => match &s.fields()[0] {
            Value::Array(array) => array,
            _ => bail!("Invalid clipboard format list"),
        },
        _ => bail!("Invalid clipboard format list"),
    };
    ensure!(array.len() <= 256, "Too many clipboard formats");
    array
        .iter()
        .map(|v| {
            let Value::Str(s) = v else {
                bail!("Invalid clipboard format name")
            };
            ensure!(s.len() <= 255, "Clipboard format name is too long");
            Ok(s.as_str().to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn documented_and_gnome_wrapped_formats_decode_without_silent_signal_loss() {
        let expected = vec!["text/plain".to_owned(), "image/png".to_owned()];
        assert_eq!(
            mime_types(&Value::from(vec!["text/plain", "image/png"])).unwrap(),
            expected
        );
        assert_eq!(
            mime_types(&Value::from((vec!["text/plain", "image/png"],))).unwrap(),
            expected
        );
        assert!(mime_types(&Value::from((vec!["text/plain"], true))).is_err());
        assert!(mime_types(&Value::from(vec![1u8, 2])).is_err());
    }
}
