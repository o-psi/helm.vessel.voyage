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
    let mut object_children = 0;
    let mut directory_children = 0;
    for grant in grants {
        // Object/container inheritance and inherited markers are understood.
        // An inherit-only ACE cannot establish access to this object.
        if grant.sid != expected || grant.flags & !0x1b != 0 || grant.mask & !ALL != 0 {
            return false;
        }
        if grant.flags & 8 == 0 {
            effective |= grant.mask;
        }
        if grant.flags & 1 != 0 {
            object_children |= grant.mask;
        }
        if grant.flags & 2 != 0 {
            directory_children |= grant.mask;
        }
    }
    effective & ALL == ALL
        && (!directory || (object_children & ALL == ALL && directory_children & ALL == ALL))
}

/// Validate the complete SID extent before native SID functions can read it.
pub(crate) fn sid_length(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 8 || bytes[0] != 1 || bytes[1] > 15 {
        return None;
    }
    let length = 8 + usize::from(bytes[1]) * 4;
    (length <= bytes.len()).then_some(length)
}
