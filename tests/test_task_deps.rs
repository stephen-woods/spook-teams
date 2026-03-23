/// Integration test 13.2: task dependency DAG with claim/complete/unblock cycle
use std::sync::Arc;
use tempfile::TempDir;

use spook_teams::db::{member, task, task_dep, team, Db};

fn setup_db() -> (Arc<Db>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Arc::new(Db::open(&db_path).unwrap());
    (db, tmp)
}

fn create_team_and_tasks(db: &Db) -> (String, Vec<String>) {
    let team_rec = {
        let conn = db.writer.lock().unwrap();
        team::create(&conn, "team", "change", "/tmp", "/tmp/tasks.md").unwrap()
    };

    let task_ids: Vec<String> = (1..=4)
        .map(|i| {
            let conn = db.writer.lock().unwrap();
            let t = task::create(
                &conn,
                &team_rec.id,
                &format!("1.{}", i),
                &format!("Task {}", i),
                None,
                None,
                task::TaskStatus::Pending,
            )
            .unwrap();
            t.id
        })
        .collect();

    (team_rec.id, task_ids)
}

#[test]
fn test_add_dependency_and_check_blocked() {
    let (db, _tmp) = setup_db();
    let (_team_id, task_ids) = create_team_and_tasks(&db);

    // Task 2 depends on task 1
    {
        let conn = db.writer.lock().unwrap();
        task_dep::add_dependency(&conn, &task_ids[1], &task_ids[0]).unwrap();
    }

    // Mark task 2 as blocked
    {
        let conn = db.writer.lock().unwrap();
        task::update_status(&conn, &task_ids[1], task::TaskStatus::Blocked).unwrap();
    }

    // Verify task 2 is blocked
    let conn = db.readers.get().unwrap();
    let t2 = task::get(&conn, &task_ids[1]).unwrap().unwrap();
    assert_eq!(t2.status, task::TaskStatus::Blocked);

    // Task 1 is still pending
    let t1 = task::get(&conn, &task_ids[0]).unwrap().unwrap();
    assert_eq!(t1.status, task::TaskStatus::Pending);
}

#[test]
fn test_claim_task() {
    let (db, _tmp) = setup_db();
    let (team_id, task_ids) = create_team_and_tasks(&db);

    let worker = {
        let conn = db.writer.lock().unwrap();
        member::create(&conn, &team_id, "alice", member::MemberRole::Worker).unwrap()
    };

    // Claim task 1
    {
        let conn = db.writer.lock().unwrap();
        task::claim(&conn, &task_ids[0], &worker.id).unwrap();
    }

    let conn = db.readers.get().unwrap();
    let t1 = task::get(&conn, &task_ids[0]).unwrap().unwrap();
    assert_eq!(t1.status, task::TaskStatus::InProgress);
    assert_eq!(t1.owner_id, Some(worker.id.clone()));
}

#[test]
fn test_complete_task_and_unblock_dependents() {
    let (db, _tmp) = setup_db();
    let (team_id, task_ids) = create_team_and_tasks(&db);

    // task 2 depends on task 1 and is blocked
    {
        let conn = db.writer.lock().unwrap();
        task_dep::add_dependency(&conn, &task_ids[1], &task_ids[0]).unwrap();
        task::update_status(&conn, &task_ids[1], task::TaskStatus::Blocked).unwrap();
    }

    let worker = {
        let conn = db.writer.lock().unwrap();
        member::create(&conn, &team_id, "alice", member::MemberRole::Worker).unwrap()
    };

    // Claim and complete task 1
    {
        let conn = db.writer.lock().unwrap();
        task::claim(&conn, &task_ids[0], &worker.id).unwrap();
        task::update_status(&conn, &task_ids[0], task::TaskStatus::Completed).unwrap();
    }

    // Compute newly unblocked: task 2 should be unblocked
    let conn = db.readers.get().unwrap();
    let unblocked = task_dep::compute_newly_unblocked(&conn, &task_ids[0]).unwrap();
    assert!(
        unblocked.contains(&task_ids[1]),
        "task 2 should be newly unblocked when task 1 completes"
    );
}

#[test]
fn test_cycle_detection_via_add_dependency() {
    let (db, _tmp) = setup_db();
    let (_team_id, task_ids) = create_team_and_tasks(&db);

    // Add A -> B
    {
        let conn = db.writer.lock().unwrap();
        task_dep::add_dependency(&conn, &task_ids[1], &task_ids[0]).unwrap();
    }

    // Try B -> A (would create cycle): should return an error
    let result = {
        let conn = db.writer.lock().unwrap();
        task_dep::add_dependency(&conn, &task_ids[0], &task_ids[1])
    };
    assert!(
        result.is_err(),
        "B -> A should be rejected as it creates a cycle"
    );
}

#[test]
fn test_available_tasks_filter_excludes_blocked() {
    let (db, _tmp) = setup_db();
    let (team_id, task_ids) = create_team_and_tasks(&db);

    // Block task 2
    {
        let conn = db.writer.lock().unwrap();
        task_dep::add_dependency(&conn, &task_ids[1], &task_ids[0]).unwrap();
        task::update_status(&conn, &task_ids[1], task::TaskStatus::Blocked).unwrap();
    }

    let conn = db.readers.get().unwrap();
    let available = task::list(&conn, &team_id, task::TaskFilter::Available).unwrap();

    let avail_ids: Vec<&str> = available.iter().map(|t| t.id.as_str()).collect();
    assert!(
        !avail_ids.contains(&task_ids[1].as_str()),
        "blocked task should not be available"
    );
    assert!(
        avail_ids.contains(&task_ids[0].as_str()),
        "task 1 should be available"
    );
}

#[test]
fn test_get_dependencies_and_dependents() {
    let (db, _tmp) = setup_db();
    let (_team_id, task_ids) = create_team_and_tasks(&db);

    // Build chain: 1 <- 2 <- 3 (task 2 depends on 1, task 3 depends on 2)
    {
        let conn = db.writer.lock().unwrap();
        task_dep::add_dependency(&conn, &task_ids[1], &task_ids[0]).unwrap();
        task_dep::add_dependency(&conn, &task_ids[2], &task_ids[1]).unwrap();
    }

    let conn = db.readers.get().unwrap();

    // task 2 dependencies = [task 1]
    let deps = task_dep::get_dependencies(&conn, &task_ids[1]).unwrap();
    assert_eq!(deps, vec![task_ids[0].clone()]);

    // task 2 dependents = [task 3]
    let dependents = task_dep::get_dependents(&conn, &task_ids[1]).unwrap();
    assert_eq!(dependents, vec![task_ids[2].clone()]);

    // task 1 is unblocked (no dependencies)
    let unblocked = task_dep::is_unblocked(&conn, &task_ids[0]).unwrap();
    assert!(unblocked, "task 1 has no deps so it is unblocked");

    // task 2 is blocked (task 1 not completed)
    let blocked = task_dep::is_unblocked(&conn, &task_ids[1]).unwrap();
    assert!(!blocked, "task 2 depends on task 1 which is not completed");
}
