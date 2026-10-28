use uuid::Uuid;

use super::*;

fn person(n: u128) -> PersonId {
    PersonId::new(Uuid::from_u128(n))
}

const CALLER: u128 = 1;
const SOMEONE: u128 = 2;

const ROLES: [Role; 2] = [Role::Administrator, Role::Member];
const OWNERS: [Owner; 3] = [
    Owner::Unowned,
    Owner::Person(PersonId::new(Uuid::from_u128(CALLER))),
    Owner::Person(PersonId::new(Uuid::from_u128(SOMEONE))),
];
const GRANTS: [Option<Level>; 3] = [None, Some(Level::Viewer), Some(Level::Editor)];

fn caller(role: Role) -> Caller {
    Caller {
        person: person(CALLER),
        role,
    }
}

/// The folder rule written out case by case, to check the function against.
fn expected_folder(role: Role, owner: Owner, granted: Option<Level>) -> Option<FolderAccess> {
    match (owner, role) {
        (Owner::Unowned, Role::Administrator) => Some(FolderAccess::Ownerless),
        (Owner::Unowned, Role::Member) => None,
        (Owner::Person(owner), _) if owner == person(CALLER) => Some(FolderAccess::Owner),
        (Owner::Person(_), _) => match (granted, role) {
            (None, _) => None,
            (Some(Level::Editor), Role::Administrator) => Some(FolderAccess::Editor),
            (Some(Level::Editor | Level::Viewer), _) => Some(FolderAccess::Viewer),
        },
    }
}

#[test]
fn every_folder_owner_grant_and_role_combination_decides_one_access() {
    for role in ROLES {
        for owner in OWNERS {
            for granted in GRANTS {
                let standing = FolderStanding { owner, granted };

                assert_eq!(
                    folder_access(caller(role), standing),
                    expected_folder(role, owner, granted),
                    "role {role:?}, owner {owner:?}, granted {granted:?}"
                );
            }
        }
    }
}

#[test]
fn a_member_owns_their_own_folder_and_is_capped_at_viewer_on_any_other() {
    let own = FolderStanding {
        owner: Owner::Person(person(CALLER)),
        granted: None,
    };
    let shared = FolderStanding {
        owner: Owner::Person(person(SOMEONE)),
        granted: Some(Level::Editor),
    };

    assert_eq!(
        folder_access(caller(Role::Member), own),
        Some(FolderAccess::Owner)
    );
    assert_eq!(
        folder_access(caller(Role::Member), shared),
        Some(FolderAccess::Viewer)
    );
}

fn folder_by(owner: u128, granted: Option<Level>) -> FolderStanding {
    FolderStanding {
        owner: Owner::Person(person(owner)),
        granted,
    }
}

/// A caller's role, a dashboard's owner, grants and carrying folders, and the
/// access they come to.
type Case = (
    Role,
    Owner,
    Option<Level>,
    Vec<FolderStanding>,
    Option<Access>,
);

fn dashboard_cases() -> Vec<Case> {
    let editor_folder = folder_by(CALLER, None);
    let viewer_folder = folder_by(SOMEONE, Some(Level::Viewer));
    let unseen_folder = folder_by(SOMEONE, None);
    let ownerless_folder = FolderStanding {
        owner: Owner::Unowned,
        granted: None,
    };
    let admin = Role::Administrator;
    let member = Role::Member;
    let mine = Owner::Person(person(CALLER));
    let theirs = Owner::Person(person(SOMEONE));
    let viewer = Some(Level::Viewer);
    let editor = Some(Level::Editor);

    vec![
        (admin, Owner::Unowned, None, vec![], Some(Access::Ownerless)),
        (
            admin,
            Owner::Unowned,
            editor,
            vec![editor_folder],
            Some(Access::Ownerless),
        ),
        (member, Owner::Unowned, editor, vec![editor_folder], None),
        (admin, mine, None, vec![], Some(Access::Owner)),
        (
            admin,
            mine,
            viewer,
            vec![viewer_folder],
            Some(Access::Owner),
        ),
        (member, mine, None, vec![], None),
        (member, mine, editor, vec![], Some(Access::Viewer)),
        (admin, theirs, None, vec![], None),
        (admin, theirs, viewer, vec![], Some(Access::Viewer)),
        (admin, theirs, editor, vec![], Some(Access::Editor)),
        (member, theirs, editor, vec![], Some(Access::Viewer)),
        (member, theirs, viewer, vec![], Some(Access::Viewer)),
        (
            admin,
            theirs,
            None,
            vec![viewer_folder],
            Some(Access::Viewer),
        ),
        (
            admin,
            theirs,
            None,
            vec![editor_folder],
            Some(Access::Editor),
        ),
        (
            admin,
            theirs,
            viewer,
            vec![editor_folder],
            Some(Access::Editor),
        ),
        (
            admin,
            theirs,
            editor,
            vec![viewer_folder],
            Some(Access::Editor),
        ),
        (
            admin,
            theirs,
            None,
            vec![viewer_folder, editor_folder],
            Some(Access::Editor),
        ),
        (
            member,
            theirs,
            None,
            vec![editor_folder],
            Some(Access::Viewer),
        ),
        (admin, theirs, None, vec![unseen_folder], None),
        (admin, theirs, None, vec![ownerless_folder], None),
        (member, theirs, None, vec![ownerless_folder], None),
    ]
}

#[test]
fn every_dashboard_combination_decides_one_access() {
    let cases = dashboard_cases();

    for (role, owner, granted, carried_by, expected) in cases {
        let standing = DashboardStanding {
            owner,
            granted,
            carried_by: carried_by.clone(),
        };

        assert_eq!(
            access(caller(role), &standing),
            expected,
            "role {role:?}, owner {owner:?}, granted {granted:?}, carried by {carried_by:?}"
        );
    }
}

#[test]
fn a_member_never_holds_more_than_viewer_on_a_dashboard() {
    for owner in OWNERS {
        for granted in GRANTS {
            let standing = DashboardStanding {
                owner,
                granted,
                carried_by: vec![folder_by(CALLER, None), folder_by(SOMEONE, granted)],
            };

            let seen = access(caller(Role::Member), &standing);

            assert!(
                matches!(seen, None | Some(Access::Viewer)),
                "owner {owner:?}, granted {granted:?}: {seen:?}"
            );
        }
    }
}

#[test]
fn the_dashboard_operation_matrix() {
    let cases = [
        (Access::Owner, Ok(()), Ok(())),
        (Access::Editor, Ok(()), Err(Refused::NotOwner)),
        (
            Access::Viewer,
            Err(Refused::NotEditor),
            Err(Refused::NotOwner),
        ),
        (Access::Ownerless, Ok(()), Ok(())),
    ];

    for (access, change, delete) in cases {
        assert_eq!(access.change(), change, "change as {access:?}");
        assert_eq!(access.delete(), delete, "delete as {access:?}");
    }
}

#[test]
fn only_the_owner_and_an_editor_by_their_own_grant_share_a_dashboard() {
    let cases = [
        (Access::Owner, None, Ok(())),
        (Access::Owner, Some(Level::Viewer), Ok(())),
        (Access::Editor, Some(Level::Editor), Ok(())),
        (
            Access::Editor,
            Some(Level::Viewer),
            Err(Refused::EditorByFolder),
        ),
        (Access::Editor, None, Err(Refused::EditorByFolder)),
        (Access::Viewer, Some(Level::Viewer), Err(Refused::NotEditor)),
        (Access::Viewer, None, Err(Refused::NotEditor)),
        (Access::Ownerless, None, Err(Refused::Ownerless)),
    ];

    for (access, granted, expected) in cases {
        assert_eq!(
            access.share(granted),
            expected,
            "{access:?} granted {granted:?}"
        );
    }
}

#[test]
fn only_the_owner_and_a_granted_editor_make_a_carrying_entry() {
    let cases = [
        (Access::Owner, None, Entry::Carrying),
        (Access::Ownerless, None, Entry::Carrying),
        (Access::Editor, Some(Level::Editor), Entry::Carrying),
        (Access::Editor, Some(Level::Viewer), Entry::Filing),
        (Access::Editor, None, Entry::Filing),
        (Access::Viewer, Some(Level::Viewer), Entry::Filing),
        (Access::Viewer, Some(Level::Editor), Entry::Filing),
        (Access::Viewer, None, Entry::Filing),
    ];

    for (access, granted, expected) in cases {
        assert_eq!(
            access.entry(granted),
            expected,
            "{access:?} granted {granted:?}"
        );
    }
}

#[test]
fn the_folder_operation_matrix() {
    let cases = [
        (FolderAccess::Owner, Ok(()), Ok(()), Ok(())),
        (FolderAccess::Editor, Ok(()), Ok(()), Err(Refused::NotOwner)),
        (
            FolderAccess::Viewer,
            Err(Refused::NotEditor),
            Err(Refused::NotEditor),
            Err(Refused::NotOwner),
        ),
        (
            FolderAccess::Ownerless,
            Ok(()),
            Err(Refused::Ownerless),
            Ok(()),
        ),
    ];

    for (access, arrange, share, delete) in cases {
        assert_eq!(access.arrange(), arrange, "arrange as {access:?}");
        assert_eq!(access.share(), share, "share as {access:?}");
        assert_eq!(access.delete(), delete, "delete as {access:?}");
    }
}

#[test]
fn an_ownerless_dashboard_and_an_ownerless_folder_go_only_together() {
    let dashboards = [
        Access::Owner,
        Access::Editor,
        Access::Viewer,
        Access::Ownerless,
    ];
    let folders = [
        FolderAccess::Owner,
        FolderAccess::Editor,
        FolderAccess::Viewer,
        FolderAccess::Ownerless,
    ];

    for dashboard in dashboards {
        for folder in folders {
            let ownerless = (
                dashboard == Access::Ownerless,
                folder == FolderAccess::Ownerless,
            );
            let expected = match ownerless {
                (true, true) | (false, false) => Ok(()),
                (true, false) => Err(Shelf::OwnerlessDashboard),
                (false, true) => Err(Shelf::OwnerlessFolder),
            };

            assert_eq!(
                shelved(dashboard, folder),
                expected,
                "{dashboard:?} in {folder:?}"
            );
        }
    }
}

#[test]
fn a_grantee_reads_back_as_the_text_it_was_parsed_from() {
    let one = "00000000-0000-4000-8000-000000000001";

    for raw in ["everyone", one] {
        let grantee = Grantee::parse(raw).unwrap_or_else(|| panic!("should parse: {raw}"));

        assert_eq!(grantee.to_string(), raw);
    }
    for refused in ["", "Everyone", "all", "not-a-uuid"] {
        assert_eq!(Grantee::parse(refused), None, "should refuse: {refused:?}");
    }
}

#[test]
fn a_level_reads_back_as_the_word_it_was_parsed_from() {
    for level in [Level::Viewer, Level::Editor] {
        assert_eq!(Level::parse(level.as_str()), Some(level));
    }
    assert_eq!(Level::parse("owner"), None);
    assert!(Level::Viewer < Level::Editor);
}
