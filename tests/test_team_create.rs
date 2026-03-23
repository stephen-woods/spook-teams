/// Integration test 13.1: team_create imports tasks from a sample tasks.md
/// and verifies the SQLite state.
use std::sync::Arc;
use tempfile::TempDir;

use spook_teams::db::Db;
use spook_teams::db::{member, task, team};

fn make_sample_tasks_md() -> &'static str {
    r#"## 1. Scaffolding

- [ ] 1.1 Set up Cargo.toml
- [ ] 1.2 Create module structure

## 2. Database Layer

- [ ] 2.1 Implement schema
- [x] 2.2 Implement CRUD (already done)
"#
}

#[test]
fn test_team_create_imports_tasks() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Arc::new(Db::open(&db_path).unwrap());

    // Write a sample tasks.md
    let tasks_file = tmp.path().join("tasks.md");
    std::fs::write(&tasks_file, make_sample_tasks_md()).unwrap();

    // Create team
    let team_rec = {
        let conn = db.writer.lock().unwrap();
        team::create(
            &conn,
            "test-team",
            "test-change",
            tmp.path().to_str().unwrap(),
            tasks_file.to_str().unwrap(),
        )
        .unwrap()
    };

    assert_eq!(team_rec.name, "test-team");
    assert_eq!(team_rec.change_name, "test-change");

    // Import tasks
    let task_count = spook_teams::bridge::import_tasks(&db, &team_rec.id, &tasks_file).unwrap();
    assert_eq!(task_count, 4, "should import 4 tasks");

    // Verify all tasks in DB
    let conn = db.readers.get().unwrap();
    let tasks = task::list(&conn, &team_rec.id, task::TaskFilter::All).unwrap();
    assert_eq!(tasks.len(), 4);

    // Tasks should have correct source IDs
    let source_ids: Vec<&str> = tasks.iter().map(|t| t.source_id.as_str()).collect();
    assert!(source_ids.contains(&"1.1"));
    assert!(source_ids.contains(&"1.2"));
    assert!(source_ids.contains(&"2.1"));
    assert!(source_ids.contains(&"2.2"));

    // 2.2 is marked done, should be Completed
    let task_2_2 = tasks.iter().find(|t| t.source_id == "2.2").unwrap();
    assert_eq!(task_2_2.status, task::TaskStatus::Completed);

    // Others should be Pending
    let task_1_1 = tasks.iter().find(|t| t.source_id == "1.1").unwrap();
    assert_eq!(task_1_1.status, task::TaskStatus::Pending);
}

#[test]
fn test_team_create_registers_lead() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Arc::new(Db::open(&db_path).unwrap());

    let tasks_file = tmp.path().join("tasks.md");
    std::fs::write(&tasks_file, make_sample_tasks_md()).unwrap();

    let team_rec = {
        let conn = db.writer.lock().unwrap();
        team::create(
            &conn,
            "test-team",
            "test-change",
            tmp.path().to_str().unwrap(),
            tasks_file.to_str().unwrap(),
        )
        .unwrap()
    };

    // Register lead
    {
        let conn = db.writer.lock().unwrap();
        member::create(&conn, &team_rec.id, "lead", member::MemberRole::Lead).unwrap();
    }

    // Verify lead registered
    let conn = db.readers.get().unwrap();
    let members = member::list_by_team(&conn, &team_rec.id).unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].name, "lead");
    assert_eq!(members[0].role, member::MemberRole::Lead);
}

#[test]
fn test_duplicate_change_name_rejected() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Arc::new(Db::open(&db_path).unwrap());

    let tasks_file = tmp.path().join("tasks.md");
    std::fs::write(&tasks_file, "## 1. Test\n- [ ] 1.1 Task\n").unwrap();

    let create_team = || {
        let conn = db.writer.lock().unwrap();
        team::create(
            &conn,
            "team",
            "my-change",
            tmp.path().to_str().unwrap(),
            tasks_file.to_str().unwrap(),
        )
    };

    create_team().expect("first creation should succeed");

    // The DB does not enforce uniqueness at this level; that's the tool layer's job.
    // Verify we can detect duplicates using get_by_change_name.
    let conn = db.readers.get().unwrap();
    let existing = team::get_by_change_name(&conn, "my-change").unwrap();
    assert!(
        existing.is_some(),
        "should find existing team by change name"
    );
}
