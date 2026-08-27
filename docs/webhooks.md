# Inbound Message & Webhook Gateway Guide 🔔

The Inbound Webhook Gateway allows external servers, monitoring daemons (Prometheus Alertmanager, Grafana, Uptime Kuma, Sentry), cron scripts, and CI/CD pipelines to send formatted alert messages and notifications into Matrix rooms.

It also provides **drop-in Telegram Bot API compatibility** (`/bot:token/sendMessage`), allowing you to use existing Telegram notification tools directly with your Matrix bot without changing any code.

---

## 🚀 Quick Start

### 1. Create a Webhook

You can create a webhook using either the Web UI or a Matrix chat command.

#### Option A: Web UI
1. Open the Web Dashboard at [http://localhost:5173](http://localhost:5173) (or your deployment URL).
2. Click **Webhooks** in the sidebar under *Integrations*.
3. Click **+ Create Webhook**, enter a name (e.g. `Production VPS Alerts`), and select the target Matrix room.
4. **Copy the generated secret token** (`whk_...`). For security, it is stored as a SHA-256 hash and will not be displayed again.

#### Option B: Matrix Chat
In a room or direct message with the bot:
```text
!webhook create "VPS Monitor" !your_room_id:matrix.org
```
The bot will reply with your webhook ID, default room, and private secret token.

---

## 📡 Sending Messages & Alerts

### 1. Standard JSON API (`POST /api/v1/webhooks/:token`)

Send a JSON payload with custom severity badges and Markdown formatting.

```bash
curl -X POST "http://localhost:8080/api/v1/webhooks/whk_YOUR_TOKEN_HERE" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Database Connection Pool Exhausted",
    "message": "PostgreSQL instance on `vps-db-01` reached **98%** connection capacity.",
    "level": "critical"
  }'
```

#### JSON Payload Fields

| Field | Type | Description |
|---|---|---|
| `message` / `text` | string | **Required**. The message body (supports Markdown and code blocks). |
| `title` | string | *Optional*. Bold header title displayed on the alert card. |
| `level` / `severity` | string | *Optional*. `critical` (🔴), `warning` (🟡), `info` (🔵), or `success` (🟢). Default: `info`. |
| `room_id` | string | *Optional*. Override the webhook's default room destination. |
| `format` | string | *Optional*. `markdown` (default) or `plain`. |

---

### 2. Simple Plaintext Fallback (`POST /api/v1/webhooks/:token`)

You can send raw plaintext directly without JSON headers:

```bash
curl -X POST "http://localhost:8080/api/v1/webhooks/whk_YOUR_TOKEN_HERE" \
  -d "Backup completed successfully in 4m 12s."
```

---

### 3. Telegram Bot API Drop-in Emulation (`/bot:token/sendMessage`)

Use your Matrix bot as a drop-in replacement for Telegram bots. Many monitoring tools and CLI utilities have built-in Telegram support.

#### POST Request:
```bash
curl -X POST "http://localhost:8080/botwhk_YOUR_TOKEN_HERE/sendMessage" \
  -H "Content-Type: application/json" \
  -d '{
    "text": "🚨 *Disk Space Alert*\nRoot partition on `/dev/sda1` is at *92%*.",
    "parse_mode": "Markdown"
  }'
```

#### GET Request (Query Parameters):
```bash
curl "http://localhost:8080/botwhk_YOUR_TOKEN_HERE/sendMessage?text=Host+rebooted+successfully"
```

---

## 🛠️ Practical Integration Recipes

### 1. VPS Server Down / Systemd Failure Alert
Trigger an alert when any critical systemd service fails:

Create `/etc/systemd/system/notify-matrix-failure@.service`:
```ini
[Unit]
Description=Send failure notification to Matrix on %i failure

[Service]
Type=oneshot
ExecStart=/usr/local/bin/notify-matrix-alert "Service %i has failed on $(hostname)" "critical"
```

Add `/usr/local/bin/notify-matrix-alert`:
```bash
#!/usr/bin/env bash
MESSAGE="$1"
LEVEL="${2:-warning}"

curl -s -X POST "https://your-bot-domain.com/api/v1/webhooks/whk_YOUR_TOKEN_HERE" \
  -H "Content-Type: application/json" \
  -d "{
    \"title\": \"Systemd Service Failure\",
    \"message\": \"${MESSAGE}\",
    \"level\": \"${LEVEL}\"
  }"
```

---

### 2. Uptime Kuma / Status Page Integration
1. In Uptime Kuma, go to **Settings** -> **Notifications** -> **Add Notification**.
2. Select **Webhook** or **Telegram**:
   - **If Webhook**: Set URL to `https://your-bot-domain.com/api/v1/webhooks/whk_YOUR_TOKEN_HERE` with POST method.
   - **If Telegram**: Set Bot Token to `whk_YOUR_TOKEN_HERE` and Custom Telegram API Server URL to `https://your-bot-domain.com/bot`.
3. Save and test.

---

### 3. Prometheus Alertmanager Configuration
Add a webhook receiver to `/etc/alertmanager/alertmanager.yml`:

```yaml
receivers:
  - name: 'matrix-alerts'
    webhook_configs:
      - url: 'https://your-bot-domain.com/api/v1/webhooks/whk_YOUR_TOKEN_HERE'
        send_resolved: true
```

---

### 4. Python Notification Script

```python
import requests

def send_matrix_alert(title: str, message: str, level: str = "info"):
    webhook_url = "https://your-bot-domain.com/api/v1/webhooks/whk_YOUR_TOKEN_HERE"
    payload = {
        "title": title,
        "message": message,
        "level": level
    }
    response = requests.post(webhook_url, json=payload, timeout=10)
    response.raise_for_status()

# Example usage:
send_matrix_alert(
    title="Deployment Finished",
    message="Production release `v1.4.2` deployed successfully.",
    level="success"
)
```

---

## 📋 Chat Commands Reference

| Command | Description | Example |
|---|---|---|
| `!webhook list` | Lists all your active and disabled webhooks with IDs and telemetry | `!webhook list` |
| `!webhook create <name> [room]` | Creates a new webhook and generates a secret token | `!webhook create "Uptime Kuma" !ops:matrix.org` |
| `!webhook test <id>` | Sends a test ping alert to the webhook's bound room | `!webhook test 1` |
| `!webhook revoke <id>` | Permanently revokes and deletes a webhook | `!webhook revoke 1` |

---

## 📜 Message History & Delivery Logs

Every message dispatched to a webhook is saved to the SQLite database.

You can browse past messages directly in the Web UI:
1. Navigate to **Webhooks** under *Integrations*.
2. Click the **History** button on any webhook card.
3. Inspect delivery statuses (🟢 Delivered, 🔴 Failed, 🟡 In Queue), severity badges, timestamps, raw payload text, and error reasons (if any).

API access for logs is also available:
```bash
GET /api/v1/account/webhooks/:id/logs?limit=50&offset=0
```

---

## 🔒 Security & Reliability Architecture

1. **SHA-256 Token Storage**:
   - Raw tokens (`whk_<hex>`) are generated using CSPRNG and displayed only once. Only the SHA-256 hash is persisted in the database.
2. **Durable Outbox Queue**:
   - Inbound requests return `202 Accepted` immediately upon writing to the transactional SQLite outbox queue.
   - Background workers dispatch alerts with instant $< 50\text{ms}$ wakeup and retry with exponential backoff on homeserver rate limits or transient outages.
3. **Rate Limiting**:
   - Protects against abuse with a sliding-window rate limit (max 60 requests per minute per webhook token).
4. **Passkey Step-Up Authentication**:
   - Sensitive management operations on the Web UI (secret rotation, deletion) require recent Passkey verification.
