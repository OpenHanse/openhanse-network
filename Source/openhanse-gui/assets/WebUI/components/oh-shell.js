class OpenHanseShell extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.commandHandler = null;
    this.shadowRoot.innerHTML = `
      <style>
        :host {
          display: block;
          min-height: 100vh;
          padding: 24px;
        }

        .frame {
          display: grid;
          grid-template-rows: auto auto 1fr auto;
          min-height: calc(100vh - 48px);
          max-width: 1100px;
          margin: 0 auto;
          border: 1px solid rgba(140, 255, 194, 0.35);
          border-radius: 14px;
          overflow: hidden;
          background:
            linear-gradient(180deg, rgba(10, 24, 17, 0.98), rgba(3, 7, 5, 0.98)),
            repeating-linear-gradient(
              180deg,
              rgba(255, 255, 255, 0.03) 0,
              rgba(255, 255, 255, 0.03) 1px,
              transparent 1px,
              transparent 4px
            );
          box-shadow:
            0 28px 80px rgba(0, 0, 0, 0.45),
            inset 0 0 0 1px rgba(140, 255, 194, 0.08);
        }

        .chrome {
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 16px;
          padding: 12px 16px;
          border-bottom: 1px solid rgba(140, 255, 194, 0.2);
          background: rgba(118, 255, 173, 0.06);
          color: #9adfb5;
          text-transform: uppercase;
          letter-spacing: 0.12em;
          font-size: 12px;
        }

        .lights {
          display: flex;
          gap: 8px;
        }

        .light {
          width: 10px;
          height: 10px;
          border-radius: 50%;
          background: currentColor;
          opacity: 0.8;
        }

        .lights .light:nth-child(1) {
          color: #ff6b6b;
        }

        .lights .light:nth-child(2) {
          color: #ffd166;
        }

        .lights .light:nth-child(3) {
          color: #7ae582;
        }

        .identity {
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
        }

        .footer {
          padding: 8px 16px;
          border-top: 1px solid rgba(140, 255, 194, 0.15);
          color: rgba(154, 223, 181, 0.72);
          font-size: 12px;
          letter-spacing: 0.08em;
          text-transform: uppercase;
          background: rgba(118, 255, 173, 0.04);
        }

        @media (max-width: 640px) {
          :host {
            padding: 12px;
          }

          .frame {
            min-height: calc(100vh - 24px);
            border-radius: 10px;
          }
        }
      </style>
      <div class="frame">
        <div class="chrome">
          <div class="lights">
            <span class="light"></span>
            <span class="light"></span>
            <span class="light"></span>
          </div>
          <div class="identity">openhanse gateway terminal</div>
        </div>
        <oh-status></oh-status>
        <oh-log></oh-log>
        <oh-prompt></oh-prompt>
        <div class="footer">local shell session</div>
      </div>
    `;
  }

  connectedCallback() {
    this.prompt.addEventListener("command", (event) => {
      if (this.commandHandler) {
        this.commandHandler(event.detail.value);
      }
    });
  }

  get statusPanel() {
    return this.shadowRoot.querySelector("oh-status");
  }

  get logPanel() {
    return this.shadowRoot.querySelector("oh-log");
  }

  get prompt() {
    return this.shadowRoot.querySelector("oh-prompt");
  }

  setApiBase(apiBase) {
    this.statusPanel.setStaticLine(`API ${apiBase}`);
  }

  setStatus(status) {
    this.statusPanel.setStatus(status);
  }

  setInbox(inbox) {
    this.statusPanel.setInboxCount(inbox.length);
  }

  appendEvent(event) {
    this.logPanel.appendEntry({
      kind: event.kind,
      message: event.message,
      timestamp: event.created_at_unix_ms
    });
  }

  appendLog(kind, message) {
    this.logPanel.appendEntry({ kind, message });
  }

  echoCommand(command) {
    const trimmed = command.trim();
    if (!trimmed) {
      return;
    }
    this.logPanel.appendEntry({
      kind: "command",
      message: trimmed
    });
  }

  clearLog() {
    this.logPanel.clear();
  }

  onCommand(handler) {
    this.commandHandler = handler;
  }

  setFatalError(message) {
    this.appendLog("error", message);
  }
}

customElements.define("oh-shell", OpenHanseShell);
