//! Restricting the database file to its owner, on both platforms.
//!
//! This is the only thing standing between `providers.api_key` and any
//! other account on the machine, so the rationale for a file mode rather
//! than a keychain is written out in full on `harden_permissions`.

use std::path::{Path, PathBuf};

/// Restrict the database and its directory to the owning user.
///
/// `providers.api_key` is the upstream credential, and it is written to disk in
/// the clear.
///
/// **That is a decision, not a deferral.** An OS keychain is read per process and
/// gated on the reading binary's signature, and this credential is needed by
/// `kiwanod` — a separate daemon that has to keep serving with the desktop app
/// closed. A keychain read from there either raises a system prompt a daemon
/// cannot answer (it would hang instead), or makes the daemon depend on the app
/// being open, which breaks the one property the whole design rests on. A key the
/// daemon *can* read unattended is a file on the same disk as this database, which
/// is the protection already in place — dressed up as encryption, but the
/// ciphertext and its key would sit in the same directory.
///
/// So the realistic leak is not a determined local attacker — anyone who can read
/// a file the app must decrypt unattended can read the key too — but the mundane
/// ones: another account on a shared machine, a backup or sync client that sweeps
/// `$HOME`, a support bundle. 0700/0600 costs nothing and closes them, and the
/// Windows ACL below closes them there.
///
/// The directory mode carries most of the weight: `~/.kiwano` also holds
/// `backups/` (the agent configs takeover replaced, credentials included) and
/// `logs/`, and 0700 stops traversal into all of it whatever the files inside
/// are set to. The per-file modes are the second line — they survive someone
/// loosening the directory, and cover a database placed outside it entirely.
///
/// Windows has no POSIX modes at all: a fresh file inherits the parent's ACL,
/// so on a stock `%USERPROFILE%` the database is readable by every account on
/// the machine (and by SYSTEM/Administrators, which the POSIX branch does not
/// exclude either, but which a shared home directory very much does). The
/// Windows branch below replaces the DACL with a single ACE for the token's
/// user and protects it, which is the same restriction the unix branch gets
/// from 0700/0600.
///
/// Best-effort by design: a filesystem without POSIX modes is not a reason to
/// refuse to start. Note the directory chmod follows `KIWANO_DB_PATH`, so a
/// database pointed somewhere unusual narrows that directory too.
pub(crate) fn harden_permissions(db_path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let owner_only = |path: &Path, mode: u32| {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
        };

        if let Some(dir) = db_path.parent() {
            owner_only(dir, 0o700);
        }
        owner_only(db_path, 0o600);
        // Both exist by now — `journal_mode = WAL` above created them — and
        // both hold copies of every credential row until the next checkpoint.
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", db_path.display()));
            if sidecar.exists() {
                owner_only(&sidecar, 0o600);
            }
        }
    }
    #[cfg(windows)]
    harden_permissions_windows(db_path);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = db_path;
    }
}

/// Windows counterpart of the unix branch above: replace the DACL of the
/// database, its WAL sidecars and the containing directory with one full-control
/// ACE for the current user, and mark that DACL protected so nothing is
/// inherited from the parent.
///
/// Built from `EXPLICIT_ACCESS_W` + `SetEntriesInAclW` + `SetNamedSecurityInfoW`
/// rather than from an SDDL string. The SDDL route
/// (`ConvertStringSecurityDescriptorToSecurityDescriptorW`) would need the SID
/// turned into text and then parsed back, and it spells the same policy as
/// `"D:P(A;OICI;FA;;;S-1-5-21-…)"`; taking the SID straight off the process
/// token and handing it to `SetEntriesInAclW` keeps it a SID throughout and
/// states "this one principal, no inheritance" as flags the compiler checks.
/// `GetNamedSecurityInfoW` is not needed to *replace* a DACL — `SetEntriesInAclW`
/// builds one from scratch when handed a NULL old ACL, and
/// `SetNamedSecurityInfoW` takes the path directly.
///
/// `PROTECTED_DACL_SECURITY_INFORMATION` is the load-bearing flag: without it
/// the parent's inheritable ACEs are merged back into the DACL we just set, so
/// the call becomes a no-op for the exact ACEs we are trying to remove (on a
/// directory under `%USERPROFILE%`, `Users` and `Authenticated Users` are
/// inherited). A failure on any one path is logged and the rest are still
/// attempted — this is hardening, not a precondition for opening the database.
#[cfg(windows)]
fn harden_permissions_windows(db_path: &Path) {
    let Some(sid) = windows_current_user_sid() else {
        tracing::warn!(
            "could not read the current user's SID; leaving the database ACLs as they are"
        );
        return;
    };

    // Directory first: its ACE carries the inherit flags, so it is what covers
    // files created under it later (`logs/`, `backups/`, and the checkpoint
    // that recreates `-wal`). The files are then given a protected DACL of
    // their own rather than relying on that inheritance — which is what keeps
    // them covered if someone loosens the directory afterwards, the same
    // second-line role the per-file modes play on unix.
    let mut targets: Vec<(PathBuf, bool)> = Vec::new();
    if let Some(dir) = db_path.parent() {
        targets.push((dir.to_path_buf(), true));
    }
    targets.push((db_path.to_path_buf(), false));
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", db_path.display()));
        // WAL mode created both before this call; a missing one is not an error
        // (same `.exists()` guard as the unix branch — an in-memory or
        // non-WAL database legitimately has neither).
        if sidecar.exists() {
            targets.push((sidecar, false));
        }
    }

    for (path, is_dir) in &targets {
        if let Err(err) = windows_set_owner_only_dacl(path, &sid, *is_dir) {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "could not restrict this path to the current user"
            );
        }
    }
}

/// The SID of the user the process runs as, as raw SID bytes.
///
/// Copied out of the `TOKEN_USER` structure rather than borrowed: the SID
/// points into the buffer `GetTokenInformation` filled, which is freed on
/// return. `None` on any failure — the caller treats that as "cannot harden"
/// and carries on.
#[cfg(windows)]
fn windows_current_user_sid() -> Option<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `token` is a valid out-pointer; the pseudo-handle from
    // `GetCurrentProcess` needs no closing.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }

    let sid = (|| {
        let mut needed: u32 = 0;
        // Sizes the buffer; the expected outcome of a NULL buffer is
        // ERROR_INSUFFICIENT_BUFFER with `needed` filled in.
        // SAFETY: a NULL buffer with length 0 is the documented sizing call.
        unsafe {
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 {
            return None;
        }

        // `u64` elements, not `u8`: `TOKEN_USER` holds pointers, so the buffer
        // has to be pointer-aligned to be read through a `*const TOKEN_USER`.
        let mut buf = vec![0u64; needed.div_ceil(8) as usize];
        let cap = (buf.len() * 8) as u32;
        // SAFETY: `buf` is `cap` bytes long and correctly aligned.
        let ok = unsafe {
            GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), cap, &mut needed)
        };
        if ok == 0 {
            return None;
        }

        // SAFETY: the call above succeeded, so `buf` holds an aligned,
        // fully-initialized `TOKEN_USER` whose `Sid` points inside it.
        let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
        if user.User.Sid.is_null() {
            return None;
        }
        // SAFETY: a token's SID is a valid SID, so this returns its length.
        let len = unsafe { GetLengthSid(user.User.Sid) } as usize;
        if len == 0 {
            return None;
        }
        // SAFETY: `Sid` points at `len` readable bytes inside `buf`, and the
        // copy happens before `buf` is dropped.
        Some(unsafe { std::slice::from_raw_parts(user.User.Sid.cast::<u8>(), len) }.to_vec())
    })();

    // SAFETY: `token` was opened by the successful `OpenProcessToken` above.
    unsafe {
        CloseHandle(token);
    }
    sid
}

/// Replace `path`'s DACL with a single full-control ACE for `sid`, protected
/// against inheritance. `is_dir` adds the container/object inherit flags so
/// children created later inherit the restriction too.
#[cfg(windows)]
fn windows_set_owner_only_dacl(path: &Path, sid: &[u8], is_dir: bool) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL},
        Security::Authorization::{
            SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
            NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
        },
        Security::{
            ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
            OBJECT_INHERIT_ACE, PROTECTED_DACL_SECURITY_INFORMATION,
        },
        Storage::FileSystem::FILE_ALL_ACCESS,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: if is_dir {
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
        } else {
            NO_INHERITANCE
        },
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            // Not a string despite the `PWSTR` type name: with
            // `TRUSTEE_IS_SID` this points at the SID bytes.
            ptstrName: sid.as_ptr() as *mut u16,
        },
    };

    // A NULL old ACL means "this list is the whole DACL", which is what makes
    // the inherited entries disappear once the result is applied protected.
    let mut new_acl: *mut ACL = std::ptr::null_mut();
    // SAFETY: `entry` is a valid EXPLICIT_ACCESS_W that outlives the call, and
    // `new_acl` is a valid out-pointer. `sid` outlives the call, and the
    // trustee holds a pointer to it.
    let rc = unsafe { SetEntriesInAclW(1, &entry, std::ptr::null(), &mut new_acl) };
    if rc != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(rc as i32));
    }

    // SAFETY: `wide` is NUL-terminated and alive for the call, the owner/group
    // and SACL pointers are deliberately null (only the DACL is being set),
    // and `new_acl` came from `SetEntriesInAclW` above.
    let rc = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            new_acl,
            std::ptr::null(),
        )
    };
    // SAFETY: `new_acl` is owned by us — `SetEntriesInAclW` allocated it with
    // LocalAlloc — and must be freed whether or not the call above succeeded.
    unsafe {
        LocalFree(new_acl as HLOCAL);
    }
    if rc != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(rc as i32));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;
    use crate::store::Store;

    /// The database carries every upstream credential, and `-wal`/`-shm` carry
    /// copies of those rows until the next checkpoint.
    #[cfg(unix)]
    #[test]
    fn open_restricts_the_database_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        // tempdir() already hands back 0700, so loosen it first — otherwise
        // the directory assertion would pass without the code under test.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.path().join("kiwano.db");
        let _store = Store::open(&path).expect("open store");

        let mode =
            |p: &std::path::Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o777;

        assert_eq!(mode(dir.path()), 0o700, "data directory");
        assert_eq!(mode(&path), 0o600, "database");
        // Both are created by journal_mode=WAL above, so both must be present
        // — the assertion is what would catch hardening running too early.
        for suffix in ["-wal", "-shm"] {
            let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
            assert!(sidecar.exists(), "{suffix} missing");
            assert_eq!(mode(&sidecar), 0o600, "sidecar {suffix}");
        }
    }

    /// Windows half of the test above: the DACL ends up with exactly one ACE,
    /// for the user this process runs as, and it is not inherited.
    ///
    /// The loosening is deliberate, as on unix: a tempdir under `%TEMP%`
    /// inherits `SYSTEM`/`Administrators`/`Users` from the profile, but that is
    /// whatever the runner happens to have, so an explicit Everyone ACE is
    /// granted first. Without it the "nothing but the owner" assertion could
    /// pass on a machine whose parent directory was already tight.
    #[cfg(windows)]
    #[test]
    fn open_restricts_the_database_to_the_owner_windows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("kiwano.db");
        std::fs::write(&path, b"").expect("create database file");
        for target in [dir.path(), path.as_path()] {
            test_acl::grant_everyone(target);
        }

        let user = windows_current_user_sid().expect("token user SID");
        for target in [dir.path(), path.as_path()] {
            let aces = test_acl::dacl(target);
            assert!(
                aces.iter().any(|(sid, _)| sid != &user),
                "{} was not widened before the open, so the assertion below would be vacuous: \
                 {aces:?}",
                target.display()
            );
        }

        let _store = Store::open(&path).expect("open store");

        for target in [dir.path(), path.as_path()] {
            let aces = test_acl::dacl(target);
            assert_eq!(aces.len(), 1, "{} ACEs: {aces:?}", target.display());
            assert_eq!(aces[0].0, user, "{} is owner-only", target.display());
            assert!(!aces[0].1, "{} must not inherit", target.display());
        }
        // Both are created by journal_mode=WAL above, so both must be present
        // — the assertion is what would catch hardening running too early.
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
            assert!(sidecar.exists(), "{suffix} missing");
            let aces = test_acl::dacl(&sidecar);
            assert_eq!(aces.len(), 1, "{suffix} ACEs: {aces:?}");
            assert_eq!(aces[0].0, user, "{suffix} is owner-only");
            assert!(!aces[0].1, "{suffix} must not inherit");
        }
    }

    /// The Windows ACL calls the test above needs on top of what production
    /// uses: reading a DACL back, and widening one on purpose.
    #[cfg(windows)]
    mod test_acl {
        use super::*;
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL},
            Security::Authorization::{
                GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
                GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, TRUSTEE_IS_SID,
                TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
            },
            Security::{
                AclSizeInformation, CreateWellKnownSid, GetAce, GetAclInformation, GetLengthSid,
                WinWorldSid, ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION,
                DACL_SECURITY_INFORMATION, INHERITED_ACE,
            },
            Storage::FileSystem::FILE_ALL_ACCESS,
        };

        /// Every ACE of `path`'s DACL as (SID bytes, inherited?). A NULL DACL
        /// — "everyone, unrestricted" — is reported as one empty SID so a
        /// caller cannot read it as "no ACEs, therefore restricted".
        pub fn dacl(path: &Path) -> Vec<(Vec<u8>, bool)> {
            let wide = wide(path);
            let mut sd = std::ptr::null_mut();
            let mut acl: *mut ACL = std::ptr::null_mut();
            // SAFETY: `wide` is NUL-terminated and outlives the call; owner,
            // group and SACL are deliberately not requested; both out-pointers
            // are valid.
            let rc = unsafe {
                GetNamedSecurityInfoW(
                    wide.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut acl,
                    std::ptr::null_mut(),
                    &mut sd,
                )
            };
            assert_eq!(
                rc,
                ERROR_SUCCESS,
                "GetNamedSecurityInfoW {}",
                path.display()
            );
            if acl.is_null() {
                // SAFETY: `sd` came from the successful call above.
                unsafe {
                    LocalFree(sd as HLOCAL);
                }
                return vec![(Vec::new(), false)];
            }

            let mut info = ACL_SIZE_INFORMATION {
                AceCount: 0,
                AclBytesInUse: 0,
                AclBytesFree: 0,
            };
            // SAFETY: `acl` is non-null and `info` is the right struct/size for
            // AclSizeInformation.
            let ok = unsafe {
                GetAclInformation(
                    acl,
                    (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                    std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                    AclSizeInformation,
                )
            };
            assert_ne!(ok, 0, "GetAclInformation {}", path.display());

            let mut out = Vec::new();
            for index in 0..info.AceCount {
                let mut ace = std::ptr::null_mut();
                // SAFETY: `index < AceCount`, and `ace` is a valid out-pointer.
                let ok = unsafe { GetAce(acl, index, &mut ace) };
                assert_ne!(ok, 0, "GetAce {} #{index}", path.display());
                // SAFETY: every ACE in a DACL starts with a header followed by
                // a mask and a SID; `ACCESS_ALLOWED_ACE` is that layout.
                let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
                let sid_ptr = std::ptr::addr_of!(ace.SidStart).cast::<u8>();
                // SAFETY: `sid_ptr` points at the SID inside the ACE above.
                let len = unsafe { GetLengthSid(sid_ptr.cast_mut().cast()) } as usize;
                // SAFETY: `len` is that SID's length in bytes.
                let sid = unsafe { std::slice::from_raw_parts(sid_ptr, len) }.to_vec();
                out.push((sid, ace.Header.AceFlags as u32 & INHERITED_ACE != 0));
            }
            // SAFETY: `sd` (and the `acl` inside it) came from the successful
            // GetNamedSecurityInfoW above and has been read fully by now.
            unsafe {
                LocalFree(sd as HLOCAL);
            }
            out
        }

        /// Grant everyone full control of `path`, without protecting the DACL
        /// — so the parent's inheritable ACEs merge back in, which is the
        /// inherited state `harden_permissions` has to remove.
        pub fn grant_everyone(path: &Path) {
            let mut sid = vec![0u8; 68]; // SECURITY_MAX_SID_SIZE
            let mut len = sid.len() as u32;
            // SAFETY: the buffer is SECURITY_MAX_SID_SIZE bytes, which holds
            // any well-known SID, and `len` is its size.
            let ok = unsafe {
                CreateWellKnownSid(
                    WinWorldSid,
                    std::ptr::null_mut(),
                    sid.as_mut_ptr().cast(),
                    &mut len,
                )
            };
            assert_ne!(ok, 0, "CreateWellKnownSid");
            sid.truncate(len as usize);

            let wide = wide(path);
            let entry = EXPLICIT_ACCESS_W {
                grfAccessPermissions: FILE_ALL_ACCESS,
                grfAccessMode: GRANT_ACCESS,
                grfInheritance: 0, // NO_INHERITANCE: the ACE applies here only
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: std::ptr::null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                    ptstrName: sid.as_ptr() as *mut u16,
                },
            };

            let mut new_acl: *mut ACL = std::ptr::null_mut();
            // SAFETY: as in `windows_set_owner_only_dacl`; `sid` outlives both
            // calls because it is owned here.
            let rc = unsafe { SetEntriesInAclW(1, &entry, std::ptr::null(), &mut new_acl) };
            assert_eq!(rc, ERROR_SUCCESS, "SetEntriesInAclW");
            let rc = unsafe {
                SetNamedSecurityInfoW(
                    wide.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    new_acl,
                    std::ptr::null(),
                )
            };
            // SAFETY: allocated by SetEntriesInAclW, owned here.
            unsafe {
                LocalFree(new_acl as HLOCAL);
            }
            assert_eq!(rc, ERROR_SUCCESS, "SetNamedSecurityInfoW");
        }

        fn wide(path: &Path) -> Vec<u16> {
            path.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }
    }
}
