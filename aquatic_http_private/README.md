# aquatic_private

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
    IN p_user_agent TEXT,
    IN p_user_token VARCHAR(255),
    IN p_info_hash CHAR(40),
    IN p_peer_id CHAR(40),
    IN p_event VARCHAR(9),
    IN p_uploaded BIGINT UNSIGNED,
    IN p_downloaded BIGINT UNSIGNED,
    IN p_left BIGINT UNSIGNED,
    OUT p_announce_allowed BOOLEAN,
    OUT p_failure_reason TEXT,
    OUT p_warning_message TEXT
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
cargo run -p aquatic_http_private
```

Test by visiting `localhost:3000/abcd/announce/?info_hash=abcdeabcdeabcdeabcde&peer_id=abcdeabcdeabcdeabcde&port=1000&left=0`