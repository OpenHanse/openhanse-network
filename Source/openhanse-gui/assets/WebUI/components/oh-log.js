class OpenHanseLog extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.entries = [];
    this.shadowRoot.innerHTML = `
      <style>
        :host {
          display: block;
          overflow: auto;
          min-height: 280px;
          padding: 12px 16px;
          white-space: pre-wrap;
          color: #d7fbe8;
        }

        #lines {
          display: grid;
          gap: 8px;
        }

        .line {
          display: grid;
          grid-template-columns: auto auto 1fr;
          gap: 12px;
          align-items: start;
        }

        .timestamp,
        .kind {
          color: rgba(154, 223, 181, 0.72);
          white-space: nowrap;
        }

        .kind {
          width: 84px;
          text-transform: uppercase;
        }

        .message {
          min-width: 0;
          word-break: break-word;
        }

        .line.command .kind,
        .line.command .message {
          color: #8df6b0;
        }

        .line.error .kind,
        .line.error .message {
          color: #ff9b9b;
        }

        .line.help .kind,
        .line.help .message,
        .line.lookup .kind,
        .line.lookup .message,
        .line.connect .kind,
        .line.connect .message {
          color: #8bd3ff;
        }

        .line.hint .kind,
        .line.hint .message {
          color: #ffe08a;
        }
      </style>
      <div id="lines"></div>
    `;
  }

  appendEntry(entry) {
    this.entries.push({
      kind: entry.kind ?? "log",
      message: entry.message ?? "",
      timestamp: entry.timestamp ?? Date.now()
    });
    this.render();
  }

  clear() {
    this.entries = [];
    this.render();
  }

  render() {
    const container = this.shadowRoot.querySelector("#lines");
    container.innerHTML = "";

    for (const entry of this.entries) {
      const line = document.createElement("div");
      line.className = `line ${sanitizeClassName(entry.kind)}`;

      const timestamp = document.createElement("span");
      timestamp.className = "timestamp";
      timestamp.textContent = formatTime(entry.timestamp);

      const kind = document.createElement("span");
      kind.className = "kind";
      kind.textContent = `[${entry.kind}]`;

      const message = document.createElement("div");
      message.className = "message";
      message.textContent = entry.message;

      line.append(timestamp, kind, message);
      container.append(line);
    }

    this.scrollTop = this.scrollHeight;
  }
}

function formatTime(value) {
  return new Date(value).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false
  });
}

function sanitizeClassName(value) {
  return String(value).replace(/[^a-z0-9_-]+/gi, "-").toLowerCase();
}

customElements.define("oh-log", OpenHanseLog);
