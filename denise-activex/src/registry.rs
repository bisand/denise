//! What `DllRegisterServer` writes, as data rather than as code.
//!
//! Self-registration is a list of registry values and nothing else, and getting
//! one of them wrong is how a control ends up invisible in a host's toolbox with
//! no error anywhere. Describing it as a table means the table can be tested on
//! any machine, including the ones with no registry at all — which is every
//! machine this repository is developed on.

/// The control's class id.
///
/// Generated once and never changed: a host stores this in its form, so a new
/// one on every build would break every project that ever embedded the control.
pub const CLSID_TEXT: &str = "{7F1B483A-5853-4348-9081-D5BD502B51E8}";

/// The versioned programmatic id, which is what a host writes in a form file.
pub const PROG_ID: &str = "Denise.Panel.1";

/// The version-independent programmatic id, for `CreateObject("Denise.Panel")`.
pub const VERSION_INDEPENDENT_PROG_ID: &str = "Denise.Panel";

/// What a host shows in its toolbox and its object browser.
pub const FRIENDLY_NAME: &str = "Denise Panel Control";

/// The control's version, as the registry spells it.
pub const VERSION: &str = "1.0";

/// The type library's id, as text.
///
/// The same value as `typelib::LIBID_DENISE`, which a test checks. `RegisterTypeLib`
/// writes the library's own keys but not this one: the link from the *class* to
/// its library is the server's to make, and without it a host that finds the class
/// has no way to discover the description.
pub const LIBID_TEXT: &str = "{5CA2EE57-C922-483E-8FDA-B0A8B3D3B195}";

/// `OLEMISC_` flags describing how a container should treat this control.
///
/// - `RECOMPOSEONRESIZE` (1): the content depends on the size, so redraw rather
///   than scale a stale bitmap.
/// - `CANTLINKINSIDE` (16): it is a control, not a linkable document.
/// - `INSIDEOUT` (128) and `ACTIVATEWHENVISIBLE` (256): activate as soon as it is
///   shown rather than waiting for a double click, which is what makes a control
///   respond to the mouse in a running form.
/// - `SETCLIENTSITEFIRST` (131072): the container must call `SetClientSite`
///   before loading persisted state. VB6 relies on it.
pub const MISC_STATUS: u32 = 1 | 16 | 128 | 256 | 131_072;

/// One registry value to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Key path below `HKEY_CLASSES_ROOT`.
    pub key: String,
    /// Value name, empty for the key's default value.
    pub name: String,
    /// The value, always a string: every value a control registers is one.
    pub value: String,
}

impl Entry {
    fn new(key: impl Into<String>, name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Every value `DllRegisterServer` writes, in the order it writes them.
///
/// `server_path` is the full path of the DLL, which a self-registering server
/// learns from `GetModuleFileName` rather than assuming.
pub fn entries(server_path: &str) -> Vec<Entry> {
    let clsid = format!("CLSID\\{CLSID_TEXT}");
    vec![
        // The two ProgIDs, both ways round. A host that only knows the name has
        // to be able to reach the CLSID, and one that only knows the CLSID has to
        // be able to name it.
        Entry::new(VERSION_INDEPENDENT_PROG_ID, "", FRIENDLY_NAME),
        Entry::new(
            format!("{VERSION_INDEPENDENT_PROG_ID}\\CLSID"),
            "",
            CLSID_TEXT,
        ),
        Entry::new(
            format!("{VERSION_INDEPENDENT_PROG_ID}\\CurVer"),
            "",
            PROG_ID,
        ),
        Entry::new(PROG_ID, "", FRIENDLY_NAME),
        Entry::new(format!("{PROG_ID}\\CLSID"), "", CLSID_TEXT),
        // The class itself.
        Entry::new(&clsid, "", FRIENDLY_NAME),
        Entry::new(format!("{clsid}\\InprocServer32"), "", server_path),
        // Apartment, not Both or Free. Every OLE control interface here assumes
        // it runs on the thread that created the window, because the window
        // procedure does.
        Entry::new(
            format!("{clsid}\\InprocServer32"),
            "ThreadingModel",
            "Apartment",
        ),
        Entry::new(format!("{clsid}\\ProgID"), "", PROG_ID),
        Entry::new(
            format!("{clsid}\\VersionIndependentProgID"),
            "",
            VERSION_INDEPENDENT_PROG_ID,
        ),
        // The empty `Control` key is the whole difference between "a COM class"
        // and "something a host will put in its toolbox".
        Entry::new(format!("{clsid}\\Control"), "", ""),
        Entry::new(format!("{clsid}\\Programmable"), "", ""),
        Entry::new(format!("{clsid}\\Version"), "", VERSION),
        Entry::new(format!("{clsid}\\TypeLib"), "", LIBID_TEXT),
        Entry::new(format!("{clsid}\\MiscStatus"), "", "0"),
        // Aspect 1 is DVASPECT_CONTENT, the only one this control draws.
        Entry::new(
            format!("{clsid}\\MiscStatus\\1"),
            "",
            MISC_STATUS.to_string(),
        ),
        Entry::new(
            format!("{clsid}\\ToolboxBitmap32"),
            "",
            format!("{server_path}, 1"),
        ),
    ]
}

/// The keys `DllUnregisterServer` deletes, deepest first.
///
/// Deepest first because the registry will not delete a key that still has
/// subkeys, and an unregister that half works leaves a control a host can still
/// see and no longer load.
pub fn keys_to_remove() -> Vec<String> {
    let clsid = format!("CLSID\\{CLSID_TEXT}");
    vec![
        format!("{clsid}\\MiscStatus\\1"),
        format!("{clsid}\\MiscStatus"),
        format!("{clsid}\\ToolboxBitmap32"),
        format!("{clsid}\\TypeLib"),
        format!("{clsid}\\Version"),
        format!("{clsid}\\Programmable"),
        format!("{clsid}\\Control"),
        format!("{clsid}\\VersionIndependentProgID"),
        format!("{clsid}\\ProgID"),
        format!("{clsid}\\InprocServer32"),
        clsid,
        format!("{PROG_ID}\\CLSID"),
        PROG_ID.to_string(),
        format!("{VERSION_INDEPENDENT_PROG_ID}\\CurVer"),
        format!("{VERSION_INDEPENDENT_PROG_ID}\\CLSID"),
        VERSION_INDEPENDENT_PROG_ID.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "C:\\Program Files\\Denise\\denise_activex.dll";

    fn value_of(entries: &[Entry], key: &str, name: &str) -> Option<String> {
        entries
            .iter()
            .find(|e| e.key == key && e.name == name)
            .map(|e| e.value.clone())
    }

    /// The four values that decide whether a host can load the control at all.
    /// Each of them fails silently: the class is simply not there.
    #[test]
    fn the_class_is_registered_where_a_host_looks_for_it() {
        let entries = entries(PATH);
        let clsid = format!("CLSID\\{CLSID_TEXT}");

        assert_eq!(
            value_of(&entries, &format!("{clsid}\\InprocServer32"), ""),
            Some(PATH.to_string()),
            "without the server path there is nothing to load"
        );
        assert_eq!(
            value_of(
                &entries,
                &format!("{clsid}\\InprocServer32"),
                "ThreadingModel"
            ),
            Some("Apartment".to_string()),
            "the window procedure is thread-affine; anything else is a race"
        );
        assert!(
            entries.iter().any(|e| e.key == format!("{clsid}\\Control")),
            "without the Control key a host will not offer it in a toolbox"
        );
        assert_eq!(
            value_of(&entries, &format!("{clsid}\\ProgID"), ""),
            Some(PROG_ID.to_string())
        );
    }

    /// A host stores the ProgID in a form file and resolves it to a CLSID later,
    /// or the other way round. Both directions have to agree or a saved form
    /// stops opening.
    #[test]
    fn the_prog_ids_and_the_clsid_agree_in_both_directions() {
        let entries = entries(PATH);
        assert_eq!(
            value_of(&entries, &format!("{PROG_ID}\\CLSID"), ""),
            Some(CLSID_TEXT.to_string())
        );
        assert_eq!(
            value_of(
                &entries,
                &format!("{VERSION_INDEPENDENT_PROG_ID}\\CLSID"),
                ""
            ),
            Some(CLSID_TEXT.to_string())
        );
        assert_eq!(
            value_of(
                &entries,
                &format!("{VERSION_INDEPENDENT_PROG_ID}\\CurVer"),
                ""
            ),
            Some(PROG_ID.to_string())
        );
        assert!(
            PROG_ID.starts_with(VERSION_INDEPENDENT_PROG_ID),
            "the versioned ProgID must extend the version-independent one"
        );
    }

    /// `SETCLIENTSITEFIRST` is the one VB6 will not proceed without, and
    /// `ACTIVATEWHENVISIBLE` is the difference between a control that responds to
    /// the mouse and a picture of one.
    #[test]
    fn the_misc_status_flags_are_the_ones_a_windowed_control_needs() {
        assert_eq!(MISC_STATUS & 131_072, 131_072, "OLEMISC_SETCLIENTSITEFIRST");
        assert_eq!(MISC_STATUS & 256, 256, "OLEMISC_ACTIVATEWHENVISIBLE");
        assert_eq!(MISC_STATUS & 128, 128, "OLEMISC_INSIDEOUT");
        assert_eq!(MISC_STATUS & 1, 1, "OLEMISC_RECOMPOSEONRESIZE");
        assert_eq!(MISC_STATUS, 131_473);

        let entries = entries(PATH);
        assert_eq!(
            value_of(&entries, &format!("CLSID\\{CLSID_TEXT}\\MiscStatus\\1"), ""),
            Some(MISC_STATUS.to_string())
        );
    }

    /// The registry will not delete a key that still has subkeys, so an
    /// unregister in the wrong order leaves a control a host can still see and
    /// can no longer load — the worst of both.
    #[test]
    fn unregistration_removes_children_before_their_parents() {
        let keys = keys_to_remove();
        for (index, key) in keys.iter().enumerate() {
            for (other_index, other) in keys.iter().enumerate() {
                if other != key && other.starts_with(&format!("{key}\\")) {
                    assert!(
                        other_index < index,
                        "{other} is inside {key} and must be removed first"
                    );
                }
            }
        }
    }

    /// Everything registration creates, unregistration must remove. A leftover
    /// key is a class a host still offers and can no longer instantiate.
    #[test]
    fn every_key_written_is_a_key_removed() {
        let removed = keys_to_remove();
        for entry in entries(PATH) {
            assert!(
                removed.contains(&entry.key),
                "{} is registered and never unregistered",
                entry.key
            );
        }
    }

    #[test]
    fn the_clsid_is_a_well_formed_braced_guid() {
        assert_eq!(CLSID_TEXT.len(), 38);
        assert!(CLSID_TEXT.starts_with('{') && CLSID_TEXT.ends_with('}'));
        let inner = &CLSID_TEXT[1..CLSID_TEXT.len() - 1];
        let groups: Vec<&str> = inner.split('-').collect();
        assert_eq!(
            groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert!(inner.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert!(
            inner.chars().all(|c| !c.is_ascii_lowercase()),
            "the registry compares these textually in places; keep one casing"
        );
    }
}
