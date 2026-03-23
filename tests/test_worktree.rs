/// Integration test 13.4: worktree DB layer — creation, lookup by member,
/// status updates, team listing, and cleanup state transitions.
use std::sync::Arc;
use tempfile::TempDir;

use spook_teams::db::member::MemberRole;
use spook_teams::db::worktree::WorktreeStatus;
use spook_teams::db::Db;
use spook_teams::db::{member, team, worktree};

fn setup_db() -> (Arc<Db>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Arc::new(Db::open(&db_path).unwrap());
    (db, tmp)
}

fn create_team_and_member(db: &Arc<Db>, tmp: &TempDir) -> (String, String) {
    let tasks_file = tmp.path().join("tasks.md");
    std::fs::write(&tasks_file, "## 1. Test\n- [ ] 1.1 Task\n").unwrap();

    let conn = db.writer.lock().unwrap();
    let team_rec = team::create(
        &conn,
        "wt-team",
        "wt-change",
        tmp.path().to_str().unwrap(),
        tasks_file.to_str().unwrap(),
    )
    .unwrap();

    let member_rec = member::create(&conn, &team_rec.id, "alice", MemberRole::Worker).unwrap();

    (team_rec.id, member_rec.id)
}

#[test]
fn test_worktree_create_and_get() {
    let (db, tmp) = setup_db();
    let (team_id, member_id) = create_team_and_member(&db, &tmp);

    let wt = {
        let conn = db.writer.lock().unwrap();
        worktree::create(
            &conn,
            &team_id,
            &member_id,
            "/project/.worktrees/alice",
            "teams/alice",
            Some("abc123"),
        )
        .unwrap()
    };

    assert_eq!(wt.team_id, team_id);
    assert_eq!(wt.member_id, member_id);
    assert_eq!(wt.path, "/project/.worktrees/alice");
    assert_eq!(wt.branch, "teams/alice");
    assert_eq!(wt.base_commit, Some("abc123".to_string()));
    assert_eq!(wt.status, WorktreeStatus::Active);

    // Get by member
    let conn = db.readers.get().unwrap();
    let fetched = worktree::get_by_member(&conn, &member_id).unwrap();
    assert!(fetched.is_some());
    let fetched = fetched.unwrap();
    assert_eq!(fetched.id, wt.id);
    assert_eq!(fetched.branch, "teams/alice");
}

#[test]
fn test_worktree_get_by_member_returns_none_when_missing() {
    let (db, _tmp) = setup_db();

    let conn = db.readers.get().unwrap();
    let result = worktree::get_by_member(&conn, "nonexistent-member-id").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_worktree_status_update() {
    let (db, tmp) = setup_db();
    let (team_id, member_id) = create_team_and_member(&db, &tmp);

    let wt = {
        let conn = db.writer.lock().unwrap();
        worktree::create(
            &conn,
            &team_id,
            &member_id,
            "/project/.worktrees/alice",
            "teams/alice",
            None,
        )
        .unwrap()
    };

    // Update to merged
    {
        let conn = db.writer.lock().unwrap();
        worktree::update_status(&conn, &wt.id, WorktreeStatus::Merged).unwrap();
    }

    let conn = db.readers.get().unwrap();
    let updated = worktree::get_by_member(&conn, &member_id).unwrap().unwrap();
    assert_eq!(updated.status, WorktreeStatus::Merged);

    // Update to cleaned_up
    {
        let conn = db.writer.lock().unwrap();
        worktree::update_status(&conn, &wt.id, WorktreeStatus::CleanedUp).unwrap();
    }

    let conn = db.readers.get().unwrap();
    let cleaned = worktree::get_by_member(&conn, &member_id).unwrap().unwrap();
    assert_eq!(cleaned.status, WorktreeStatus::CleanedUp);
}

#[test]
fn test_worktree_update_base_commit() {
    let (db, tmp) = setup_db();
    let (team_id, member_id) = create_team_and_member(&db, &tmp);

    let wt = {
        let conn = db.writer.lock().unwrap();
        worktree::create(
            &conn,
            &team_id,
            &member_id,
            "/project/.worktrees/alice",
            "teams/alice",
            Some("initial-sha"),
        )
        .unwrap()
    };

    {
        let conn = db.writer.lock().unwrap();
        worktree::update_base_commit(&conn, &wt.id, "new-sha-after-rebase").unwrap();
    }

    let conn = db.readers.get().unwrap();
    let updated = worktree::get_by_member(&conn, &member_id).unwrap().unwrap();
    assert_eq!(
        updated.base_commit,
        Some("new-sha-after-rebase".to_string())
    );
}

#[test]
fn test_worktree_list_by_team() {
    let (db, tmp) = setup_db();

    let tasks_file = tmp.path().join("tasks.md");
    std::fs::write(&tasks_file, "## 1. Test\n- [ ] 1.1 Task\n").unwrap();

    let (team_id, member_alice, member_bob) = {
        let conn = db.writer.lock().unwrap();
        let team_rec = team::create(
            &conn,
            "multi-wt-team",
            "multi-wt-change",
            tmp.path().to_str().unwrap(),
            tasks_file.to_str().unwrap(),
        )
        .unwrap();
        let m_a = member::create(&conn, &team_rec.id, "alice", MemberRole::Worker).unwrap();
        let m_b = member::create(&conn, &team_rec.id, "bob", MemberRole::Worker).unwrap();
        (team_rec.id, m_a.id, m_b.id)
    };

    {
        let conn = db.writer.lock().unwrap();
        worktree::create(
            &conn,
            &team_id,
            &member_alice,
            "/project/.worktrees/alice",
            "teams/alice",
            None,
        )
        .unwrap();
        worktree::create(
            &conn,
            &team_id,
            &member_bob,
            "/project/.worktrees/bob",
            "teams/bob",
            None,
        )
        .unwrap();
    }

    let conn = db.readers.get().unwrap();
    let wts = worktree::list_by_team(&conn, &team_id).unwrap();
    assert_eq!(wts.len(), 2);

    let branches: Vec<&str> = wts.iter().map(|w| w.branch.as_str()).collect();
    assert!(branches.contains(&"teams/alice"));
    assert!(branches.contains(&"teams/bob"));
}

#[test]
fn test_worktree_lifecycle_active_to_merged_to_cleaned() {
    let (db, tmp) = setup_db();
    let (team_id, member_id) = create_team_and_member(&db, &tmp);

    // Create worktree — active
    let wt = {
        let conn = db.writer.lock().unwrap();
        worktree::create(
            &conn,
            &team_id,
            &member_id,
            "/project/.worktrees/alice",
            "teams/alice",
            Some("sha0"),
        )
        .unwrap()
    };
    assert_eq!(wt.status, WorktreeStatus::Active);

    // Merge
    {
        let conn = db.writer.lock().unwrap();
        worktree::update_status(&conn, &wt.id, WorktreeStatus::Merged).unwrap();
    }
    {
        let conn = db.readers.get().unwrap();
        let w = worktree::get_by_member(&conn, &member_id).unwrap().unwrap();
        assert_eq!(w.status, WorktreeStatus::Merged);
    }

    // Cleanup
    {
        let conn = db.writer.lock().unwrap();
        worktree::update_status(&conn, &wt.id, WorktreeStatus::CleanedUp).unwrap();
    }
    {
        let conn = db.readers.get().unwrap();
        let w = worktree::get_by_member(&conn, &member_id).unwrap().unwrap();
        assert_eq!(w.status, WorktreeStatus::CleanedUp);
    }
}
