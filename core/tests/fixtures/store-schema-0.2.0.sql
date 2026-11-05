-- The store's schema after its first 33 migrations, as a release wrote it
-- (core/src/store_upgrade_tests.rs; written by its test write_a_releases_schema).
CREATE TABLE settings (
         key   TEXT PRIMARY KEY NOT NULL,
         value TEXT NOT NULL
     );
CREATE TABLE identity (
         id           INTEGER PRIMARY KEY CHECK (id = 1),
         threema_id   TEXT NOT NULL,
         client_key   BLOB NOT NULL,
         server_group TEXT NOT NULL,
         created_at   INTEGER NOT NULL
     );
CREATE TABLE contacts (
        identity                 TEXT PRIMARY KEY NOT NULL,
        public_key               TEXT NOT NULL,
        created_at               INTEGER NOT NULL,
        first_name               TEXT,
        last_name                TEXT,
        nickname                 TEXT,
        verification_level       INTEGER NOT NULL,
        work_verification_level  INTEGER NOT NULL,
        identity_type            INTEGER NOT NULL,
        acquaintance_level       INTEGER NOT NULL,
        activity_state           INTEGER NOT NULL,
        feature_mask             INTEGER NOT NULL,
        sync_state               INTEGER NOT NULL,
        read_receipt_override    INTEGER,
        typing_indicator_override INTEGER,
        conversation_category    INTEGER NOT NULL,
        conversation_visibility  INTEGER NOT NULL
    );
CREATE TABLE messages (
        id            INTEGER PRIMARY KEY,
        conversation  TEXT NOT NULL,
        message_id    INTEGER NOT NULL,
        outgoing      INTEGER NOT NULL,
        kind          TEXT NOT NULL,
        body          TEXT NOT NULL,
        created_at    INTEGER NOT NULL,
        updated_at    INTEGER NOT NULL,
        state         INTEGER NOT NULL, edited_at INTEGER, sender TEXT, starred INTEGER NOT NULL DEFAULT 0, sent_at INTEGER, delivered_at INTEGER, read_at INTEGER, received_at INTEGER, sort_key INTEGER NOT NULL DEFAULT 0,
        UNIQUE (conversation, outgoing, message_id)
    );
CREATE INDEX messages_by_conversation ON messages (conversation, created_at);
CREATE TABLE nonces (
        nonce BLOB PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE fs_sessions (
        peer                 TEXT NOT NULL,
        session_id           BLOB NOT NULL,
        my_ephemeral_private BLOB,
        my_ephemeral_public  BLOB NOT NULL,
        version_local        INTEGER,
        version_remote       INTEGER,
        last_outgoing_at     INTEGER NOT NULL,
        my_2dh_key  BLOB, my_2dh_counter  INTEGER,
        my_4dh_key  BLOB, my_4dh_counter  INTEGER,
        peer_2dh_key BLOB, peer_2dh_counter INTEGER,
        peer_4dh_key BLOB, peer_4dh_counter INTEGER,
        PRIMARY KEY (peer, session_id)
    );
CREATE TABLE reactions (
        message     INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
        sender      TEXT NOT NULL,
        emoji       TEXT NOT NULL,
        reacted_at  INTEGER NOT NULL,
        PRIMARY KEY (message, sender, emoji)
    ) WITHOUT ROWID;
CREATE TABLE outbox (
        id            INTEGER PRIMARY KEY,
        conversation  TEXT NOT NULL,
        message_id    INTEGER NOT NULL,
        created_at    INTEGER NOT NULL,
        kind          TEXT NOT NULL,
        body          TEXT NOT NULL,
        UNIQUE (conversation, message_id)
    );
CREATE TABLE files (
        message            INTEGER PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
        key                BLOB NOT NULL,
        blob_id            BLOB,
        thumbnail_blob_id  BLOB
    );
CREATE TABLE groups (
        key         TEXT PRIMARY KEY NOT NULL,
        creator     TEXT NOT NULL,
        group_id    INTEGER NOT NULL,
        name        TEXT NOT NULL DEFAULT '',
        members     TEXT NOT NULL,
        left_group  INTEGER NOT NULL DEFAULT 0,
        created_at  INTEGER NOT NULL
    , picture TEXT, picture_version INTEGER NOT NULL DEFAULT 0);
CREATE TABLE pending_receivers (
        conversation  TEXT NOT NULL,
        message_id    INTEGER NOT NULL,
        receiver      TEXT NOT NULL,
        PRIMARY KEY (conversation, message_id, receiver)
    ) WITHOUT ROWID;
CREATE TABLE group_resyncs (
        key   TEXT PRIMARY KEY NOT NULL,
        at    INTEGER NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE polls (
        conversation  TEXT NOT NULL,
        creator       TEXT NOT NULL,
        poll_id       INTEGER NOT NULL,
        message       INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
        PRIMARY KEY (conversation, creator, poll_id)
    ) WITHOUT ROWID;
CREATE TABLE contact_pictures (
        identity  TEXT PRIMARY KEY NOT NULL,
        version   INTEGER NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE picture_distribution (
        identity    TEXT PRIMARY KEY NOT NULL,
        blob_id     BLOB,
        removed_at  INTEGER
    ) WITHOUT ROWID;
CREATE TABLE chat_notifications (
        conversation  TEXT PRIMARY KEY NOT NULL,
        policy        TEXT NOT NULL,
        until         INTEGER
    ) WITHOUT ROWID;
CREATE TABLE pinned_chats (
        conversation  TEXT PRIMARY KEY NOT NULL,
        position      INTEGER NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE distribution_lists (
        key         TEXT PRIMARY KEY NOT NULL,
        name        TEXT NOT NULL,
        members     TEXT NOT NULL,
        created_at  INTEGER NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE archived_chats (
        conversation  TEXT PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE picture_recipients (
        identity  TEXT PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE private_chats (
        conversation  TEXT PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE emptied_chats (
        conversation  TEXT PRIMARY KEY NOT NULL,
        at            INTEGER NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE gif_favourites (
        id        INTEGER PRIMARY KEY,
        hash      TEXT NOT NULL UNIQUE,
        width     INTEGER,
        height    INTEGER,
        size      INTEGER NOT NULL,
        added_at  INTEGER NOT NULL
    );
CREATE TABLE d2d_nonces (
        nonce BLOB PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE d2d_sync_queue (
        id       INTEGER PRIMARY KEY,
        scope    INTEGER NOT NULL,
        content  BLOB NOT NULL
    );
CREATE TABLE group_calls (
        call_id          BLOB PRIMARY KEY NOT NULL,
        conversation     TEXT NOT NULL,
        gck              BLOB NOT NULL,
        sfu_base_url     TEXT NOT NULL,
        protocol_version INTEGER NOT NULL,
        received_at      INTEGER NOT NULL,
        failed           INTEGER NOT NULL DEFAULT 0
    ) WITHOUT ROWID;
CREATE TABLE web_sessions (
        id              INTEGER PRIMARY KEY,
        initiator_key   BLOB NOT NULL,
        private_key     BLOB NOT NULL,
        server_key      BLOB,
        host            TEXT NOT NULL,
        port            INTEGER NOT NULL,
        self_hosted     INTEGER NOT NULL DEFAULT 0,
        persistent      INTEGER NOT NULL DEFAULT 0,
        client          TEXT NOT NULL DEFAULT '',
        created_at      INTEGER NOT NULL,
        last_connection INTEGER
    );
CREATE TABLE web_hosts (
        host TEXT PRIMARY KEY,
        spki TEXT
    );
CREATE INDEX messages_by_conversation_id ON messages (conversation, id);
CREATE INDEX messages_unread ON messages (conversation) WHERE outgoing = 0 AND state = 0;
CREATE TABLE picture_requests (
        identity      TEXT PRIMARY KEY NOT NULL,
        requested_at  INTEGER NOT NULL,
        attempts      INTEGER NOT NULL,
        next_at       INTEGER NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE picture_downloads (
        identity    TEXT PRIMARY KEY NOT NULL,
        blob        BLOB NOT NULL,
        key         BLOB NOT NULL,
        size        INTEGER NOT NULL,
        generation  INTEGER NOT NULL,
        attempts    INTEGER NOT NULL,
        next_at     INTEGER NOT NULL
    ) WITHOUT ROWID;
CREATE INDEX messages_position ON messages (conversation, sort_key, id);
CREATE TRIGGER messages_sort_key AFTER INSERT ON messages
        WHEN NEW.sort_key = 0
    BEGIN
        UPDATE messages SET sort_key = NEW.id << 20 WHERE id = NEW.id;
    END;
CREATE TABLE android_import_ids (
        source        TEXT NOT NULL,
        family        TEXT NOT NULL,
        conversation  TEXT NOT NULL,
        uid           TEXT NOT NULL,
        message_id    INTEGER NOT NULL,
        PRIMARY KEY (source, family, conversation, uid)
    ) WITHOUT ROWID;
CREATE VIRTUAL TABLE messages_search USING fts5(
        content, content='', contentless_delete=1, tokenize='trigram');
CREATE INDEX messages_mentions ON messages (id) WHERE body LIKE '%@[%';
CREATE TRIGGER messages_search_insert AFTER INSERT ON messages
        WHEN NEW.kind IN ('text', 'file', 'location', 'poll')
    BEGIN
        INSERT INTO messages_search (rowid, content) VALUES (NEW.id,
            CASE WHEN NEW.kind = 'text' THEN NEW.body
                 WHEN json_valid(NEW.body) THEN concat_ws(' ',
                     json_extract(NEW.body, '$.caption'), json_extract(NEW.body, '$.name'),
                     json_extract(NEW.body, '$.address'), json_extract(NEW.body, '$.description'))
                 ELSE '' END);
    END;
CREATE TRIGGER messages_search_update AFTER UPDATE OF kind, body ON messages
    BEGIN
        DELETE FROM messages_search WHERE rowid = OLD.id;
        INSERT INTO messages_search (rowid, content)
            SELECT NEW.id,
                CASE WHEN NEW.kind = 'text' THEN NEW.body
                     WHEN json_valid(NEW.body) THEN concat_ws(' ',
                         json_extract(NEW.body, '$.caption'), json_extract(NEW.body, '$.name'),
                         json_extract(NEW.body, '$.address'), json_extract(NEW.body, '$.description'))
                     ELSE '' END
            WHERE NEW.kind IN ('text', 'file', 'location', 'poll');
    END;
CREATE TRIGGER messages_search_delete AFTER DELETE ON messages
    BEGIN
        DELETE FROM messages_search WHERE rowid = OLD.id;
    END;
CREATE TABLE marked_unread_chats (
        conversation  TEXT PRIMARY KEY NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE chat_summaries (
        chat       TEXT PRIMARY KEY NOT NULL,
        last_id    INTEGER NOT NULL,
        last_sort  INTEGER NOT NULL,
        unread     INTEGER NOT NULL
    ) WITHOUT ROWID;
CREATE TRIGGER chat_summaries_insert AFTER INSERT ON messages
    BEGIN
        INSERT INTO chat_summaries (chat, last_id, last_sort, unread)
            VALUES (NEW.conversation, NEW.id, NEW.sort_key,
                    CASE WHEN NEW.outgoing = 0 AND NEW.state = 0 THEN 1 ELSE 0 END)
        ON CONFLICT(chat) DO UPDATE SET
            unread = unread + (CASE WHEN NEW.outgoing = 0 AND NEW.state = 0 THEN 1 ELSE 0 END),
            last_id = CASE WHEN NEW.sort_key > last_sort
                                OR (NEW.sort_key = last_sort AND NEW.id > last_id)
                           THEN NEW.id ELSE last_id END,
            last_sort = max(last_sort, NEW.sort_key);
    END;
CREATE TRIGGER chat_summaries_sort AFTER UPDATE OF sort_key ON messages
    BEGIN
        UPDATE chat_summaries SET last_id = NEW.id, last_sort = NEW.sort_key
         WHERE chat = NEW.conversation
           AND (NEW.sort_key > last_sort OR (NEW.sort_key = last_sort AND NEW.id > last_id));
        UPDATE chat_summaries
           SET last_id = (SELECT n.id FROM messages n WHERE n.conversation = NEW.conversation
                          ORDER BY n.sort_key DESC, n.id DESC LIMIT 1),
               last_sort = (SELECT n.sort_key FROM messages n WHERE n.conversation = NEW.conversation
                            ORDER BY n.sort_key DESC, n.id DESC LIMIT 1)
         WHERE chat = NEW.conversation AND last_id = NEW.id;
    END;
CREATE TRIGGER chat_summaries_state AFTER UPDATE OF state, outgoing ON messages
    BEGIN
        UPDATE chat_summaries
           SET unread = max(0, unread
                - (CASE WHEN OLD.outgoing = 0 AND OLD.state = 0 THEN 1 ELSE 0 END)
                + (CASE WHEN NEW.outgoing = 0 AND NEW.state = 0 THEN 1 ELSE 0 END))
         WHERE chat = NEW.conversation;
    END;
CREATE TRIGGER chat_summaries_delete AFTER DELETE ON messages
    BEGIN
        UPDATE chat_summaries
           SET unread = max(0, unread
                    - (CASE WHEN OLD.outgoing = 0 AND OLD.state = 0 THEN 1 ELSE 0 END)),
               last_id = CASE WHEN last_id = OLD.id
                              THEN coalesce((SELECT n.id FROM messages n
                                             WHERE n.conversation = OLD.conversation
                                             ORDER BY n.sort_key DESC, n.id DESC LIMIT 1), -1)
                              ELSE last_id END,
               last_sort = CASE WHEN last_id = OLD.id
                                THEN coalesce((SELECT n.sort_key FROM messages n
                                               WHERE n.conversation = OLD.conversation
                                               ORDER BY n.sort_key DESC, n.id DESC LIMIT 1), 0)
                                ELSE last_sort END
         WHERE chat = OLD.conversation;
        -- Nothing left in the chat: no summary either.
        DELETE FROM chat_summaries WHERE chat = OLD.conversation AND last_id = -1;
    END;
