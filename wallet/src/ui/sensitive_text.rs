use std::fmt;

use zeroize::{Zeroize, Zeroizing};

#[derive(Default)]
pub(super) struct SensitiveText(String);

impl SensitiveText {
    pub(super) fn replace(&mut self, value: String) {
        self.0.zeroize();
        self.0 = value;
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn snapshot(&self) -> Zeroizing<String> {
        Zeroizing::new(self.0.clone())
    }
}

impl Drop for SensitiveText {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Display for SensitiveText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
