/// Integration test 13.6: End-to-end lifecycle test at the DB/bridge layer.
///
/// Simulates the full team lifecycle:
///   1. Create team, import tasks from tasks.md
///   2. Register lead + worker agents (members)
///   3. Create worktrees for workers
///   4. Workers claim and complete tasks with dependencies
///   5. Messages are exchanged between agents
///   6. Team progress is tracked (count_by_status)
///   7. Team is marked completed at the end
///
/// OpenCode processes are NOT spawned — the spawner layer is tested separately.
use std::sync::Arc;
use tempfile::TempDir;

use spook_teams::bridge;
use spook_teams::db::member::MemberRole;
use spook_teams::db::message::MessageType;
use spook_teams::db::task::TaskStatus;
use spook_teams::db::team::TeamStatus;
use spook_teams::db::worktree::WorktreeStatus;
use spook_teams::db::Db;
use spook_teams::db::{member, message, task, task_dep, team, worktree};

/// A representative tasks.md with dependencies that exercises blocking/unblocking.
fn make_e2e_tasks_md() -> &'static str {
    r#"## 1. Foundation

- [ ] 1.1 Set up project scaffold
- [ ] 1.2 Configure database schema

## 2. Core Features

- [ ] 2.1 Implement authentication
- [ ] 2.2 Implement API endpoints
- [ ] 2.3 Write integration tests

## 3. Release

- [x] 3.1 Update changelog (already done)
- [ ] 3.2 Create release build
"#
}

fn setup() -> (Arc<Db>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("e2e.db");
    let db = Arc::new(Db::open(&db_path).unwrap());
    (db, tmp)
}

/// Full happy-path lifecycle: create team → import tasks → agents work →
/// complete all tasks → team ends.
#[test]
fn test_full_lifecycle() {
    let (db, tmp) = setup();

    // ── 1. Create team ────────────────────────────────────────────────────────
    let tasks_file = tmp.path().join("tasks.md");
    std::fs::write(&tasks_file, make_e2e_tasks_md()).unwrap();

    let team_rec = {
        let conn = db.writer.lock().unwrap();
        team::create(
            &conn,
            "e2e-team",
            "e2e-change",
            tmp.path().to_str().unwrap(),
            tasks_file.to_str().unwrap(),
        )
        .unwrap()
    };
    assert_eq!(team_rec.status, TeamStatus::Active);

    // ── 2. Import tasks ───────────────────────────────────────────────────────
    let task_count = bridge::import_tasks(&db, &team_rec.id, &tasks_file).unwrap();
    assert_eq!(task_count, 7, "7 tasks total (including pre-completed 3.1)");

    // Verify 3.1 is already completed
    {
        let conn = db.readers.get().unwrap();
        let t = task::get_by_source_id(&conn, &team_rec.id, "3.1")
            .unwrap()
            .unwrap();
        assert_eq!(t.status, TaskStatus::Completed);
    }

    // ── 3. Register lead + 2 workers ─────────────────────────────────────────
    let (lead_id, alice_id, bob_id) = {
        let conn = db.writer.lock().unwrap();
        let lead = member::create(&conn, &team_rec.id, "lead", MemberRole::Lead).unwrap();
        let alice = member::create(&conn, &team_rec.id, "alice", MemberRole::Worker).unwrap();
        let bob = member::create(&conn, &team_rec.id, "bob", MemberRole::Worker).unwrap();
        (lead.id, alice.id, bob.id)
    };

    // ── 4. Create worktrees ───────────────────────────────────────────────────
    let (wt_alice_id, wt_bob_id) = {
        let conn = db.writer.lock().unwrap();
        let wt_a = worktree::create(
            &conn,
            &team_rec.id,
            &alice_id,
            "/proj/.worktrees/alice",
            "teams/alice",
            Some("sha-main"),
        )
        .unwrap();
        let wt_b = worktree::create(
            &conn,
            &team_rec.id,
            &bob_id,
            "/proj/.worktrees/bob",
            "teams/bob",
            Some("sha-main"),
        )
        .unwrap();
        (wt_a.id, wt_b.id)
    };

    // ── 5. Set up task dependency: 2.3 depends on 2.1 and 2.2 ────────────────
    let (task_21_id, task_22_id, task_23_id) = {
        let conn = db.readers.get().unwrap();
        let t21 = task::get_by_source_id(&conn, &team_rec.id, "2.1")
            .unwrap()
            .unwrap();
        let t22 = task::get_by_source_id(&conn, &team_rec.id, "2.2")
            .unwrap()
            .unwrap();
        let t23 = task::get_by_source_id(&conn, &team_rec.id, "2.3")
            .unwrap()
            .unwrap();
        (t21.id, t22.id, t23.id)
    };

    {
        let conn = db.writer.lock().unwrap();
        task_dep::add_dependency(&conn, &task_23_id, &task_21_id).unwrap();
        task_dep::add_dependency(&conn, &task_23_id, &task_22_id).unwrap();
        // Mark 2.3 as blocked
        task::update_status(&conn, &task_23_id, TaskStatus::Blocked).unwrap();
    }

    // ── 6. Alice claims task 1.1, Bob claims task 1.2 ────────────────────────
    let task_11_id = {
        let conn = db.readers.get().unwrap();
        task::get_by_source_id(&conn, &team_rec.id, "1.1")
            .unwrap()
            .unwrap()
            .id
    };
    let task_12_id = {
        let conn = db.readers.get().unwrap();
        task::get_by_source_id(&conn, &team_rec.id, "1.2")
            .unwrap()
            .unwrap()
            .id
    };

    {
        let conn = db.writer.lock().unwrap();
        task::claim(&conn, &task_11_id, &alice_id).unwrap();
        task::claim(&conn, &task_12_id, &bob_id).unwrap();
    }

    // Verify both tasks in_progress
    {
        let conn = db.readers.get().unwrap();
        let counts = task::count_by_status(&conn, &team_rec.id).unwrap();
        assert_eq!(counts.in_progress, 2);
        // 2 pending (2.1, 2.2), 1 blocked (2.3), 1 completed (3.1), 1 pending (3.2)
        // = 3 pending + 1 blocked + 1 completed + 2 in_progress = 7
        assert_eq!(counts.total(), 7);
    }

    // ── 7. Alice and Bob exchange messages ───────────────────────────────────
    {
        let conn = db.writer.lock().unwrap();
        message::insert(
            &conn,
            &team_rec.id,
            &alice_id,
            &bob_id,
            None,
            MessageType::Text,
            "Hey Bob, I'm working on 1.1. You can start 1.2.",
            None,
        )
        .unwrap();
        message::insert(
            &conn,
            &team_rec.id,
            &bob_id,
            &alice_id,
            None,
            MessageType::Text,
            "Got it! On it.",
            None,
        )
        .unwrap();
    }

    // Verify inbox
    {
        let conn = db.readers.get().unwrap();
        let alice_inbox = message::get_inbox(&conn, &team_rec.id, &alice_id, false).unwrap();
        assert_eq!(alice_inbox.len(), 1);
        assert!(alice_inbox[0].body.contains("Got it"));

        let bob_inbox = message::get_inbox(&conn, &team_rec.id, &bob_id, false).unwrap();
        assert_eq!(bob_inbox.len(), 1);
    }

    // ── 8. Complete tasks 1.1, 1.2 ──────────────────────────────────────────
    {
        let conn = db.writer.lock().unwrap();
        task::update_status(&conn, &task_11_id, TaskStatus::Completed).unwrap();
        task::update_status(&conn, &task_12_id, TaskStatus::Completed).unwrap();
    }

    // ── 9. Alice and Bob work on 2.1 and 2.2 ────────────────────────────────
    {
        let conn = db.writer.lock().unwrap();
        task::claim(&conn, &task_21_id, &alice_id).unwrap();
        task::claim(&conn, &task_22_id, &bob_id).unwrap();
    }

    {
        let conn = db.writer.lock().unwrap();
        task::update_status(&conn, &task_21_id, TaskStatus::Completed).unwrap();
        task::update_status(&conn, &task_22_id, TaskStatus::Completed).unwrap();
    }

    // Verify 2.3 is now unblocked (both deps complete)
    {
        let conn = db.readers.get().unwrap();
        let unblocked = task_dep::is_unblocked(&conn, &task_23_id).unwrap();
        assert!(
            unblocked,
            "2.3 should be unblocked after 2.1 and 2.2 complete"
        );
    }

    // ── 10. Complete remaining tasks ─────────────────────────────────────────
    let task_32_id = {
        let conn = db.readers.get().unwrap();
        task::get_by_source_id(&conn, &team_rec.id, "3.2")
            .unwrap()
            .unwrap()
            .id
    };

    {
        let conn = db.writer.lock().unwrap();
        task::update_status(&conn, &task_23_id, TaskStatus::Pending).unwrap();
        task::claim(&conn, &task_23_id, &alice_id).unwrap();
        task::update_status(&conn, &task_23_id, TaskStatus::Completed).unwrap();

        task::claim(&conn, &task_32_id, &bob_id).unwrap();
        task::update_status(&conn, &task_32_id, TaskStatus::Completed).unwrap();
    }

    // ── 11. Verify all tasks complete ────────────────────────────────────────
    {
        let conn = db.readers.get().unwrap();
        let counts = task::count_by_status(&conn, &team_rec.id).unwrap();
        assert_eq!(counts.completed, 7, "all 7 tasks should be completed");
        assert_eq!(counts.pending, 0);
        assert_eq!(counts.in_progress, 0);
        assert_eq!(counts.blocked, 0);

        let progress = counts.progress_pct();
        assert!(
            (progress - 1.0).abs() < f32::EPSILON,
            "progress should be 100%"
        );
    }

    // ── 12. Update worktrees to merged ────────────────────────────────────────
    {
        let conn = db.writer.lock().unwrap();
        worktree::update_status(&conn, &wt_alice_id, WorktreeStatus::Merged).unwrap();
        worktree::update_status(&conn, &wt_bob_id, WorktreeStatus::Merged).unwrap();
    }

    // ── 13. End team ──────────────────────────────────────────────────────────
    {
        let conn = db.writer.lock().unwrap();
        team::update_status(&conn, &team_rec.id, TeamStatus::Completed).unwrap();
        // Mark workers completed
        member::update_status(&conn, &alice_id, member::MemberStatus::Completed).unwrap();
        member::update_status(&conn, &bob_id, member::MemberStatus::Completed).unwrap();
    }

    // ── 14. Verify final state ────────────────────────────────────────────────
    {
        let conn = db.readers.get().unwrap();
        let final_team = team::get(&conn, &team_rec.id).unwrap().unwrap();
        assert_eq!(final_team.status, TeamStatus::Completed);

        // Team should no longer appear in get_by_change_name (status is completed, not active)
        let active = team::get_by_change_name(&conn, "e2e-change").unwrap();
        assert!(
            active.is_none(),
            "completed team should not appear as active"
        );

        // Lead should still be there
        let members = member::list_by_team(&conn, &team_rec.id).unwrap();
        assert_eq!(members.len(), 3, "lead + 2 workers");

        // Worktrees should be merged
        let wts = worktree::list_by_team(&conn, &team_rec.id).unwrap();
        assert!(wts.iter().all(|w| w.status == WorktreeStatus::Merged));
    }

    // Check that we're not dropping the lead_id without using it
    let _ = lead_id;
}

/// Test that a task failing during a run is correctly reflected in counts.
#[test]
fn test_task_failure_tracked() {
    let (db, tmp) = setup();
    let tasks_file = tmp.path().join("tasks.md");
    std::fs::write(
        &tasks_file,
        "## 1. Test\n- [ ] 1.1 Task A\n- [ ] 1.2 Task B\n",
    )
    .unwrap();

    let team_rec = {
        let conn = db.writer.lock().unwrap();
        team::create(
            &conn,
            "fail-team",
            "fail-change",
            tmp.path().to_str().unwrap(),
            tasks_file.to_str().unwrap(),
        )
        .unwrap()
    };
    bridge::import_tasks(&db, &team_rec.id, &tasks_file).unwrap();

    let worker_id = {
        let conn = db.writer.lock().unwrap();
        member::create(&conn, &team_rec.id, "worker", MemberRole::Worker)
            .unwrap()
            .id
    };

    let task_id = {
        let conn = db.readers.get().unwrap();
        task::get_by_source_id(&conn, &team_rec.id, "1.1")
            .unwrap()
            .unwrap()
            .id
    };

    {
        let conn = db.writer.lock().unwrap();
        task::claim(&conn, &task_id, &worker_id).unwrap();
        task::update_status(&conn, &task_id, TaskStatus::Failed).unwrap();
    }

    {
        let conn = db.readers.get().unwrap();
        let counts = task::count_by_status(&conn, &team_rec.id).unwrap();
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.pending, 1); // 1.2 still pending
    }

    let _ = worker_id;
}

/// Test that message read/unread state is tracked correctly.
#[test]
fn test_message_read_state() {
    let (db, tmp) = setup();
    let tasks_file = tmp.path().join("tasks.md");
    std::fs::write(&tasks_file, "## 1. Test\n- [ ] 1.1 Task\n").unwrap();

    let team_rec = {
        let conn = db.writer.lock().unwrap();
        team::create(
            &conn,
            "msg-team",
            "msg-change",
            tmp.path().to_str().unwrap(),
            tasks_file.to_str().unwrap(),
        )
        .unwrap()
    };

    let (sender_id, recipient_id) = {
        let conn = db.writer.lock().unwrap();
        let s = member::create(&conn, &team_rec.id, "sender", MemberRole::Worker).unwrap();
        let r = member::create(&conn, &team_rec.id, "recipient", MemberRole::Worker).unwrap();
        (s.id, r.id)
    };

    // Insert 3 messages
    for i in 0..3 {
        let conn = db.writer.lock().unwrap();
        message::insert(
            &conn,
            &team_rec.id,
            &sender_id,
            &recipient_id,
            None,
            MessageType::Text,
            &format!("Message {}", i),
            None,
        )
        .unwrap();
    }

    // All should be unread
    {
        let conn = db.readers.get().unwrap();
        let unread = message::get_inbox(&conn, &team_rec.id, &recipient_id, true).unwrap();
        assert_eq!(unread.len(), 3);
    }

    // Mark all read (mark_read marks all messages for a recipient at once)
    {
        let conn = db.writer.lock().unwrap();
        message::mark_read(&conn, &team_rec.id, &recipient_id).unwrap();
    }

    // Now none should be unread
    {
        let conn = db.readers.get().unwrap();
        let unread = message::get_inbox(&conn, &team_rec.id, &recipient_id, true).unwrap();
        assert_eq!(unread.len(), 0);
        // But all should still appear in full inbox
        let all = message::get_inbox(&conn, &team_rec.id, &recipient_id, false).unwrap();
        assert_eq!(all.len(), 3);
    }
}
