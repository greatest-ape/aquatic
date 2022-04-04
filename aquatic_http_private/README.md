# aquatic_http_private

HTTP (over TLS) BitTorrent tracker that calls a mysql stored procedure to
determine if requests can proceed.

Work in progress.

## Usage

### Database setup

* Create database (you will typically skip this step and use your own database):

```sql
CREATE DATABASE aquatic_db;
```

* Create aquatic user (use a better password):

```sql
CREATE USER 'aquatic'@localhost IDENTIFIED BY 'aquatic_password';
```

* Create stored procedure `aquatic_announce_v1`:

```sql
-- Create stored procedure called by aquatic for each announce request.
--
-- Set output parameter p_announce_allowed determines to true to allow announce.
CREATE OR REPLACE PROCEDURE aquatic_announce_v1 (
    -- Canonical source ip address (IPv4/IPv6)
    IN p_source_ip VARBINARY(16),
    -- Source port (not port where peer says it will accept BitTorrent requests)
    IN p_source_port SMALLINT UNSIGNED,
    -- User agent (can be NULL)
    IN p_user_agent TEXT,
    -- User token extracted from announce url ('/announce/USER_TOKEN/)
    IN p_user_token VARCHAR(255),
    -- Hex-encoded info hash
    IN p_info_hash CHAR(40),
    -- Peer ID
    IN p_peer_id CHAR(40),
    -- Event (started/stopped/completed) (can be NULL)
    IN p_event VARCHAR(9),
    -- Bytes uploaded. Passed directly from request.
    IN p_uploaded BIGINT UNSIGNED,
    -- Bytes downloaded. Passed directly from request.
    IN p_downloaded BIGINT UNSIGNED,
    -- Bytes left
    IN p_left BIGINT UNSIGNED,
    -- Return true to send annonunce response. Defaults to false if not set.
    OUT p_announce_allowed BOOLEAN,
    -- Optional failure reason. Defaults to NULL if not set.
    OUT p_failure_reason TEXT,
    -- Optional warning message. Defaults to NULL if not set.
    OUT p_warning_message TEXT
)
MODIFIES SQL DATA
BEGIN
    -- Replace with your custom code
    SELECT true INTO p_announce_allowed;
END
```

* Give aquatic user permission to call stored procedure:

```sql
GRANT EXECUTE ON PROCEDURE aquatic_db.aquatic_announce_v1 TO 'aquatic'@localhost;
FLUSH PRIVILEGES;
```

`CREATE OR REPLACE PROCEDURE` command, which leaves privileges in place,
requires MariaDB 10.1.3 or later. If your database does not support it,
each time you want to replace the procedure, you need to drop it, then
create it using `CREATE PROCEDURE` and grant execution privileges again.

### Tracker setup

* Install rust compiler and cmake

* Create `.env` file with database credentials:

```sh
DATABASE_URL="mysql://aquatic:aquatic_password@localhost/aquatic_db"
```

* Build and run tracker:

```sh
# Build
cargo build --release -p aquatic_http_private
# Generate config file (remember to set paths to TLS cert and key)
./target/release/aquatic_http_private -p > http-private-config.toml
# Run tracker
./target/release/aquatic_http_private -c http-private-config.toml
```
