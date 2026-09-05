//! Deliberately narrow ACL policy. Unknown ACE forms must fail closed upstream.
#[derive(Clone)]
pub(crate) struct Grant {
    pub sid: Vec<u8>,
    pub mask: u32,
    pub flags: u8,
}
pub(crate) fn private_acl(
    owner: &[u8],
    expected: &[u8],
    protected: bool,
    directory: bool,
    grants: &[Grant],
) -> bool {
    const ALL: u32 = 0x001f01ff;
    if owner != expected || owner.is_empty() || (directory && !protected) || grants.is_empty() {
        return false;
    }
    let mut effective = 0;
    for grant in grants {
        // Object/container inheritance and inherited markers are understood.
        // An inherit-only ACE cannot establish access to this object.
        if grant.sid != expected || grant.flags & !0x1b != 0 || grant.mask & !ALL != 0 {
            return false;
        }
        if grant.flags & 8 == 0 {
            effective |= grant.mask;
        }
    }
    effective & ALL == ALL
}

#[cfg(test)]
mod tests {
    use super::*;
    const ALL: u32 = 0x001f01ff;
    fn owner_grant(flags: u8) -> Grant {
        Grant {
            sid: vec![1, 2],
            mask: ALL,
            flags,
        }
    }
    #[test]
    fn accepts_protected_owner_directory_and_inherited_owner_file() {
        assert!(private_acl(&[1, 2], &[1, 2], true, true, &[owner_grant(3)]));
        assert!(private_acl(
            &[1, 2],
            &[1, 2],
            false,
            false,
            &[owner_grant(0x10)]
        ));
    }
    #[test]
    fn rejects_foreign_ownership_broad_grants_and_mutable_inheritance() {
        assert!(!private_acl(&[9], &[1, 2], true, true, &[owner_grant(3)]));
        assert!(!private_acl(
            &[1, 2],
            &[1, 2],
            false,
            true,
            &[owner_grant(3)]
        ));
        let mut broad = owner_grant(0x10);
        broad.sid = vec![9];
        broad.mask = 1;
        assert!(!private_acl(
            &[1, 2],
            &[1, 2],
            true,
            true,
            &[owner_grant(3), broad]
        ));
    }
    #[test]
    fn rejects_empty_inherit_only_unknown_flags_and_insufficient_access() {
        assert!(!private_acl(&[1, 2], &[1, 2], true, true, &[]));
        assert!(!private_acl(
            &[1, 2],
            &[1, 2],
            true,
            true,
            &[owner_grant(8)]
        ));
        assert!(!private_acl(
            &[1, 2],
            &[1, 2],
            true,
            true,
            &[owner_grant(0x40)]
        ));
        let mut read_only = owner_grant(0);
        read_only.mask = 1;
        assert!(!private_acl(&[1, 2], &[1, 2], true, false, &[read_only]));
    }
}
