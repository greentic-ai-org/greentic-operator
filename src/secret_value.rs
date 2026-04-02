use std::{borrow::Cow, fmt};

#[derive(Clone)]
pub struct SecretValue<'a> {
    inner: Cow<'a, [u8]>,
}

impl<'a> SecretValue<'a> {
    pub fn new(value: &'a [u8]) -> Self {
        Self {
            inner: Cow::Borrowed(value),
        }
    }

    pub fn owned(value: Vec<u8>) -> SecretValue<'static> {
        SecretValue {
            inner: Cow::Owned(value),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.inner
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl<'a> From<&'a [u8]> for SecretValue<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self::new(value)
    }
}

impl From<Vec<u8>> for SecretValue<'static> {
    fn from(value: Vec<u8>) -> Self {
        Self::owned(value)
    }
}

impl fmt::Display for SecretValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Debug for SecretValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::SecretValue;

    #[test]
    fn debug_and_display_are_redacted() {
        let value = SecretValue::new(b"super-secret");
        assert_eq!(format!("{value}"), "[REDACTED]");
        assert_eq!(format!("{value:?}"), "[REDACTED]");
    }

    #[test]
    fn owned_and_borrowed_values_expose_bytes_and_lengths() {
        let borrowed = SecretValue::new(b"abc");
        assert_eq!(borrowed.as_bytes(), b"abc");
        assert_eq!(borrowed.len(), 3);
        assert!(!borrowed.is_empty());

        let owned = SecretValue::owned(Vec::new());
        assert!(owned.is_empty());
        assert_eq!(owned.len(), 0);
        assert_eq!(owned.as_bytes(), b"");
    }
}
