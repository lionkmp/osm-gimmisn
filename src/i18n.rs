/*
 * Copyright 2021 Miklos Vajna. All rights reserved.
 * Use of this source code is governed by a BSD-style license that can be
 * found in the LICENSE file.
 */

#![deny(warnings)]
#![warn(clippy::all)]
#![warn(missing_docs)]

//! The i18n module allows UI translation via gettext.

thread_local! {
    static TRANSLATIONS: std::cell::RefCell<Option<gettext::Catalog>> = std::cell::RefCell::new(None);
    static LANGUAGE: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
}

/// Sets the language of the current thread.
pub fn set_language(language: &str) -> anyhow::Result<()> {
    // Not using ctx.get_abspath() here, tests/ doesn't have its own dummy translations.
    let root_dir = env!("CARGO_MANIFEST_DIR");
    let path = format!(
        "{}/locale/{}/LC_MESSAGES/osm-gimmisn.mo",
        root_dir, language
    );

    if std::path::Path::new(&path).exists() {
        let file = std::fs::File::open(path)?;
        let catalog = gettext::Catalog::parse(file)?;
        TRANSLATIONS.with(|it| {
            *it.borrow_mut() = Some(catalog);
        });
    } else {
        TRANSLATIONS.with(|it| {
            *it.borrow_mut() = None;
        });
    }
    LANGUAGE.with(|it| {
        *it.borrow_mut() = Some(String::from(language));
    });
    Ok(())
}

/// Gets the language of the current thread.
pub fn get_language() -> String {
    LANGUAGE.with(|language| {
        let language = language.borrow();
        match *language {
            Some(ref language) => language.clone(),
            None => String::from("en"),
        }
    })
}

/// Translates English input according to the current UI language.
pub fn translate(english: &str) -> String {
    TRANSLATIONS.with(|translations| {
        let translations = translations.borrow();
        match *translations {
            Some(ref translations) => translations.gettext(english).to_string(),
            None => english.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Context manager for translate().
    struct LanguageContext {}

    impl LanguageContext {
        /// Switches to the new language.
        fn new(language: &str) -> Self {
            assert_eq!(set_language(language).is_ok(), true);
            LanguageContext {}
        }
    }

    impl Drop for LanguageContext {
        /// Switches back to the old language.
        fn drop(&mut self) {
            assert_eq!(set_language("en").is_ok(), true)
        }
    }

    /// Tests translate().
    #[test]
    fn test_translate() {
        let _lc = LanguageContext::new("hu");
        assert_eq!(translate("Area"), "Terület");
    }
}
