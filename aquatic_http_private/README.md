# aquatic_http_private

Work in progress.

## Setup

Create user:

```sql
CREATE DATABASE aquatic;
CREATE USER 'aquatic'@localhost IDENTIFIED BY 'aquatic';
GRANT EXECUTE ON PROCEDURE aquatic.aquatic_announce_v1 TO 'aquatic'@localhost;
FLUSH PRIVILEGES;
```

Create stored procedure (`OR REPLACE` keeps privileges in place and is supported by MariaDB since 10.1.3):

```sql
CREATE OR REPLACE PROCEDURE aquatic_announce_v1 (
    IN p_source_ip VARBINARY(16),
    IN p_source_port SMALLINT UNSIGNED,
    IN p_user_agent TEXT, -- Can be NULL
    IN p_user_token VARCHAR(255),
    IN p_info_hash CHAR(40),
    IN p_peer_id CHAR(40),
    IN p_event VARCHAR(9),
    IN p_uploaded BIGINT UNSIGNED,
    IN p_downloaded BIGINT UNSIGNED,
    IN p_left BIGINT UNSIGNED,
    OUT p_announce_allowed BOOLEAN, -- false if not set
    OUT p_failure_reason TEXT, -- NULL if not set
    OUT p_warning_message TEXT -- NULL if not set
)
MODIFIES SQL DATA
BEGIN
    SELECT true INTO p_announce_allowed;
END
```

Create `.env` file:

```sh
DATABASE_URL="mysql://aquatic:aquatic@localhost/aquatic"
```

Run application:

```sh
cargo run --release -p aquatic_http_private
```