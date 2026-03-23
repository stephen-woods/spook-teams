/// Integration test 13.3: message routing for direct and topic messages
use std::sync::Arc;
use tempfile::TempDir;

use spook_teams::db::{member, message, team, Db};

fn setup() -> (Arc<Db>, TempDir, String, String, String) {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Arc::new(Db::open(&db_path).unwrap());

    // Create team
    let team_rec = {
        let conn = db.writer.lock().unwrap();
        team::create(&conn, "team", "change", "/tmp", "/tmp/tasks.md").unwrap()
    };

    // Create lead + 2 workers
    let lead = {
        let conn = db.writer.lock().unwrap();
        member::create(&conn, &team_rec.id, "lead", member::MemberRole::Lead).unwrap()
    };
    let alice = {
        let conn = db.writer.lock().unwrap();
        member::create(&conn, &team_rec.id, "alice", member::MemberRole::Worker).unwrap()
    };
    let bob = {
        let conn = db.writer.lock().unwrap();
        member::create(&conn, &team_rec.id, "bob", member::MemberRole::Worker).unwrap()
    };

    (db, tmp, team_rec.id, lead.id, alice.id)
}

#[test]
fn test_direct_message_delivery() {
    let (db, _tmp, team_id, lead_id, alice_id) = setup();

    // Lead sends direct message to alice
    {
        let conn = db.writer.lock().unwrap();
        message::insert(
            &conn,
            &team_id,
            &lead_id,
            &alice_id,
            None,
            message::MessageType::Text,
            "Hello alice, please start task 1.1",
            None,
        )
        .unwrap();
    }

    // Alice reads her inbox
    let conn = db.readers.get().unwrap();
    let inbox = message::get_inbox(&conn, &team_id, &alice_id, false).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].body, "Hello alice, please start task 1.1");
    assert_eq!(inbox[0].sender_id, lead_id);
    assert!(!inbox[0].is_read);
}

#[test]
fn test_team_broadcast_delivery() {
    let (db, _tmp, team_id, alice_id, bob_id) = setup();

    // Alice broadcasts to #team
    {
        let conn = db.writer.lock().unwrap();
        message::insert(
            &conn,
            &team_id,
            &alice_id,
            "#team",
            Some("#team"),
            message::MessageType::Text,
            "I've completed my tasks!",
            None,
        )
        .unwrap();
    }

    // Both alice and bob can read the team message (it shows up in any inbox via #team)
    let conn = db.readers.get().unwrap();
    let alice_inbox = message::get_inbox(&conn, &team_id, &alice_id, false).unwrap();
    let bob_inbox = message::get_inbox(&conn, &team_id, &bob_id, false).unwrap();

    // The #team message should be visible in everyone's inbox
    assert!(
        alice_inbox.iter().any(|m| m.recipient == "#team"),
        "alice should see #team message"
    );
    assert!(
        bob_inbox.iter().any(|m| m.recipient == "#team"),
        "bob should see #team message"
    );
}

#[test]
fn test_mark_messages_read() {
    let (db, _tmp, team_id, lead_id, alice_id) = setup();

    // Send 2 messages to alice
    {
        let conn = db.writer.lock().unwrap();
        for i in 0..2 {
            message::insert(
                &conn,
                &team_id,
                &lead_id,
                &alice_id,
                None,
                message::MessageType::Text,
                &format!("Message {}", i),
                None,
            )
            .unwrap();
        }
    }

    // Verify 2 unread
    {
        let conn = db.readers.get().unwrap();
        let unread = message::get_inbox(&conn, &team_id, &alice_id, true).unwrap();
        assert_eq!(unread.len(), 2, "should have 2 unread messages");
    }

    // Mark all read
    {
        let conn = db.writer.lock().unwrap();
        let count = message::mark_read(&conn, &team_id, &alice_id).unwrap();
        assert_eq!(count, 2, "should mark 2 messages as read");
    }

    // Now unread_only should return 0
    {
        let conn = db.readers.get().unwrap();
        let unread = message::get_inbox(&conn, &team_id, &alice_id, true).unwrap();
        assert_eq!(unread.len(), 0, "no unread messages after marking read");
    }

    // But all messages still visible without filter
    {
        let conn = db.readers.get().unwrap();
        let all = message::get_inbox(&conn, &team_id, &alice_id, false).unwrap();
        assert_eq!(all.len(), 2, "all messages still visible");
        assert!(all.iter().all(|m| m.is_read), "all messages should be read");
    }
}

#[test]
fn test_message_types() {
    let (db, _tmp, team_id, lead_id, alice_id) = setup();

    let msg_types = [
        message::MessageType::TaskComplete,
        message::MessageType::MergeConflict,
        message::MessageType::Crash,
    ];

    {
        let conn = db.writer.lock().unwrap();
        for mt in &msg_types {
            message::insert(
                &conn,
                &team_id,
                &lead_id,
                &alice_id,
                None,
                mt.clone(),
                "payload",
                None,
            )
            .unwrap();
        }
    }

    let conn = db.readers.get().unwrap();
    let inbox = message::get_inbox(&conn, &team_id, &alice_id, false).unwrap();
    assert_eq!(inbox.len(), 3);

    let types: Vec<&message::MessageType> = inbox.iter().map(|m| &m.message_type).collect();
    assert!(types.contains(&&message::MessageType::TaskComplete));
    assert!(types.contains(&&message::MessageType::MergeConflict));
    assert!(types.contains(&&message::MessageType::Crash));
}
