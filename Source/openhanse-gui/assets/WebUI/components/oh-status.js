class OpenHanseStatus extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.staticLine = "";
    this.inboxCount = 0;
    this.status = null;
    this.shadowRoot.innerHTML = `
      <style>
        :host {
          display: block;
          padding: 16px;
          border-bottom: 1px solid rgba(140, 255, 194, 0.16);
          background: rgba(118, 255, 173, 0.03);
        }
        
        .grid {
          display: grid;
          grid-template-columns: repeat(2, minmax(0, 1fr));
          gap: 8px 20px;
        }

        .row {
          display: grid;
          grid-template-columns: 84px 1fr;
          gap: 12px;
          min-width: 0;
        }

        .label {
          color: rgba(154, 223, 181, 0.72);
          text-transform: uppercase;
          letter-spacing: 0.08em;
        }

        .value {
          min-width: 0;
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
          color: #d7fbe8;
        }

        .value.ok {
          color: #8df6b0;
        }

        .value.error {
          color: #ff9b9b;
        }

        .meta {
          margin-top: 10px;
          color: rgba(154, 223, 181, 0.72);
          font-size: 12px;
        }

        @media (max-width: 720px) {
          .grid {
            grid-template-columns: 1fr;
          }
        }
      </style>
      <div class="grid" id="grid"></div>
      <div class="meta" id="meta"></div>
    `;
  }

  setStaticLine(value) {
    this.staticLine = value;
    this.render();
  }

  setInboxCount(value) {
    this.inboxCount = value;
    this.render();
  }

  setStatus(status) {
    this.status = status;
    this.render();
  }

  render() {
    const grid = this.shadowRoot.querySelector("#grid");
    const meta = this.shadowRoot.querySelector("#meta");
    const rows = [];

    if (this.status) {
      rows.push(["peer", this.status.peer_id]);
      rows.push(["target", this.status.target_peer_id]);
      rows.push(["server", this.status.server_base_url]);
      rows.push(["direct", `${this.status.direct_base_url}${this.status.message_endpoint}`]);
      rows.push(["heart", this.status.heartbeat_state, heartbeatClassName(this.status.heartbeat_state)]);
      rows.push(["mode", this.status.last_delivery_mode ?? "n/a"]);
      rows.push(["inbox", String(this.inboxCount)]);

      if (this.status.last_error) {
        rows.push(["error", this.status.last_error, "error"]);
      }

      if (this.status.last_delivery_summary) {
        rows.push(["last", this.status.last_delivery_summary]);
      }

      meta.textContent = [
        this.staticLine,
        `events ${this.status.event_count}`,
        `sent d:${this.status.direct_sent_count} r:${this.status.relay_sent_count}`,
        `recv d:${this.status.direct_received_count} r:${this.status.relay_received_count}`,
        this.status.display_name ? `name ${this.status.display_name}` : ""
      ].filter(Boolean).join("  |  ");
    } else {
      meta.textContent = this.staticLine;
    }

    grid.innerHTML = "";
    for (const [labelText, valueText, valueClass = ""] of rows) {
      const row = document.createElement("div");
      row.className = "row";

      const label = document.createElement("div");
      label.className = "label";
      label.textContent = labelText;

      const value = document.createElement("div");
      value.className = `value ${valueClass}`.trim();
      value.textContent = valueText;

      row.append(label, value);
      grid.append(row);
    }
  }
}

function heartbeatClassName(value) {
  if (!value) {
    return "";
  }

  const normalized = value.toLowerCase();
  if (normalized.includes("error") || normalized.includes("fail")) {
    return "error";
  }
  if (normalized.includes("ok") || normalized.includes("healthy") || normalized.includes("registered")) {
    return "ok";
  }
  return "";
}

customElements.define("oh-status", OpenHanseStatus);
