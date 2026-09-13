//! Accepted-audience policy for session verification.
//!
//! A verifier usually accepts exactly one audience, but it may accept several —
//! for example while an audience is being renamed, or when one verifier serves
//! several distinct audiences. Membership is an exact match, never a prefix,
//! suffix, or case-insensitive one, so a token minted for one audience is never
//! accepted for another. The matched value reaches callers through
//! [`crate::claims::AuthContext::audience`], where it can inform authorization
//! decisions.

use std::collections::BTreeSet;

/// Why an [`AudienceSet`] could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AudienceSetError {
    /// A verifier that accepts nothing can never authenticate anyone, and a
    /// verifier that accepted *everything* would accept tokens minted for any
    /// audience at all. Refusing the empty set makes the misconfiguration loud
    /// at construction instead of silent at request time.
    #[error("an audience set must contain at least one audience")]
    Empty,
    /// An empty audience string would match a token whose `aud` claim was
    /// omitted or blank.
    #[error("an audience must not be blank")]
    BlankAudience,
}

/// The set of audiences a verifier will accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudienceSet(BTreeSet<String>);

impl AudienceSet {
    /// Accept exactly one audience — the common case.
    pub fn single(audience: impl Into<String>) -> Self {
        let audience = audience.into();
        Self(BTreeSet::from([audience]))
    }

    /// Accept any audience in `audiences`.
    ///
    /// Rejects an empty set and blank entries rather than defaulting to
    /// something permissive.
    pub fn new<I, S>(audiences: I) -> Result<Self, AudienceSetError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let set: BTreeSet<String> = audiences.into_iter().map(Into::into).collect();
        if set.is_empty() {
            return Err(AudienceSetError::Empty);
        }
        if set.iter().any(|audience| audience.trim().is_empty()) {
            return Err(AudienceSetError::BlankAudience);
        }
        Ok(Self(set))
    }

    /// Exact membership. Deliberately not a prefix or case-insensitive test: a
    /// loose comparison would let a token minted for one audience be accepted
    /// for another.
    ///
    /// A blank audience is never accepted, whatever the set contains.
    /// [`Self::new`] refuses blank entries, but [`Self::single`] is infallible
    /// so the existing verifier constructors can stay infallible, which leaves
    /// a blank value reachable by misconfiguration. Rejecting here closes that
    /// off at the point it matters and cannot be bypassed by any constructor:
    /// a token whose `aud` claim was omitted or empty deserializes to `""` and
    /// must never authenticate.
    pub fn accepts(&self, audience: &str) -> bool {
        if audience.trim().is_empty() {
            return false;
        }
        self.0.contains(audience)
    }

    /// The accepted audiences, in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// How many audiences are accepted. Always at least one.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always false; retained so clippy does not ask for it alongside `len`.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The sole accepted audience, when there is exactly one.
    pub fn as_single(&self) -> Option<&str> {
        if self.0.len() == 1 {
            self.0.iter().next().map(String::as_str)
        } else {
            None
        }
    }
}

impl From<String> for AudienceSet {
    fn from(audience: String) -> Self {
        Self::single(audience)
    }
}

impl From<&str> for AudienceSet {
    fn from(audience: &str) -> Self {
        Self::single(audience)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_audience_accepts_only_itself() {
        let set = AudienceSet::single("deployment-31");
        assert!(set.accepts("deployment-31"));
        assert!(!set.accepts("deployment-32"));
        assert_eq!(set.len(), 1);
        assert_eq!(set.as_single(), Some("deployment-31"));
    }

    #[test]
    fn a_set_accepts_every_member_and_nothing_else() {
        let set = AudienceSet::new(["deployment-31", "deployment-32", "deployment-40"])
            .expect("non-empty");
        for member in ["deployment-31", "deployment-32", "deployment-40"] {
            assert!(set.accepts(member), "{member} should be accepted");
        }
        assert!(!set.accepts("deployment-99"));
        assert_eq!(
            set.as_single(),
            None,
            "a set with several audiences has no single audience"
        );
    }

    #[test]
    fn membership_is_exact() {
        // A prefix or case-insensitive match would let a token minted for one
        // audience be accepted for another.
        let set = AudienceSet::single("deployment-3");
        assert!(!set.accepts("deployment-31"));
        assert!(!set.accepts("deployment"));
        assert!(!set.accepts("DEPLOYMENT-3"));
        assert!(!set.accepts(" deployment-3"));
        assert!(!set.accepts(""));
    }

    #[test]
    fn an_empty_or_blank_set_is_refused_at_construction() {
        assert_eq!(
            AudienceSet::new(Vec::<String>::new()).unwrap_err(),
            AudienceSetError::Empty
        );
        assert_eq!(
            AudienceSet::new([""]).unwrap_err(),
            AudienceSetError::BlankAudience
        );
        assert_eq!(
            AudienceSet::new(["deployment-31", "   "]).unwrap_err(),
            AudienceSetError::BlankAudience
        );
    }

    #[test]
    fn a_blank_audience_never_authenticates_even_if_the_set_holds_one() {
        // `single` is infallible so the scalar verifier constructors can be,
        // which means a misconfigured deployment can build a set around "".
        // A signed token carrying `aud: ""` must still be refused.
        let set = AudienceSet::single("");
        assert!(!set.accepts(""));
        assert!(!set.accepts("   "));

        // The same holds for a set that legitimately contains other audiences.
        let set = AudienceSet::new(["deployment-31"]).expect("non-empty");
        assert!(!set.accepts(""));
        assert!(set.accepts("deployment-31"));
    }

    #[test]
    fn duplicates_collapse() {
        let set = AudienceSet::new(["deployment-31", "deployment-31"]).expect("non-empty");
        assert_eq!(set.len(), 1);
    }
}
