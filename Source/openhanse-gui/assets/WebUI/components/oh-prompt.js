class OpenHansePrompt extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.shadowRoot.innerHTML = `
      <style>
        :host {
          display: block;
          padding: 12px 16px 16px;
        }

        form {
          display: grid;
          grid-template-columns: auto 1fr;
          gap: 8px;
          align-items: center;
        }

        .prompt {
          color: #8df6b0;
        }

        input {
          width: 100%;
          border: 0;
          padding: 0;
          font: inherit;
          background: transparent;
          color: inherit;
          outline: none;
        }
      </style>
      <form>
        <span class="prompt">guest@openhanse:~$</span>
        <input type="text" autocomplete="off" spellcheck="false" aria-label="Terminal input">
      </form>
    `;
  }

  connectedCallback() {
    const form = this.shadowRoot.querySelector("form");
    const input = this.shadowRoot.querySelector("input");
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const value = input.value;
      input.value = "";
      this.dispatchEvent(new CustomEvent("command", {
        detail: { value },
        bubbles: true,
        composed: true
      }));
    });
    input.focus();
  }
}

customElements.define("oh-prompt", OpenHansePrompt);
