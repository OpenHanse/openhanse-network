const apiBase = window.OPENHANSE_API_BASE ?? window.location.origin;
const shell = document.querySelector("oh-shell");

if (!apiBase) {
  shell.setFatalError("Missing local API base URL.");
} else {
  const state = {
    apiBase,
    sinceEventId: null,
    pollTimer: null,
    inbox: [],
    events: []
  };

  shell.setApiBase(apiBase);
  shell.appendLog("system", "OpenHanse gateway shell ready.");
  shell.appendLog("hint", "Type /help to list available commands.");
  shell.onCommand(async (input) => {
    shell.echoCommand(input);
    try {
      await handleInput(state, input);
    } catch (error) {
      shell.appendLog("error", describeError("command", error));
    }
  });

  startPolling(state).catch((error) => {
    shell.appendLog("error", describeError("initial poll", error));
  });
}

async function startPolling(state) {
  await poll(state);
  state.pollTimer = window.setInterval(() => {
    poll(state).catch((error) => {
      shell.appendLog("error", describeError("poll", error));
    });
  }, 1500);
}

async function poll(state) {
  const search = state.sinceEventId == null ? "" : `?since_event_id=${state.sinceEventId}`;
  const url = `${state.apiBase}/api/poll${search}`;
  const response = await fetch(url).catch((error) => {
    throw new Error(`GET ${url} failed: ${error.message}`);
  });
  const payload = await response.json();

  state.inbox = payload.inbox ?? [];
  shell.setStatus(payload.status);
  shell.setInbox(state.inbox);

  for (const event of payload.events ?? []) {
    shell.appendEvent(event);
    state.sinceEventId = event.id;
  }
}

async function handleInput(state, rawInput) {
  const input = rawInput.trim();
  if (!input) {
    return;
  }

  if (!input.startsWith("/")) {
    const response = await postJSON(`${state.apiBase}/api/messages`, { message: input });
    shell.appendLog(response.delivery_mode, `Sent message to ${response.target_url}`);
    return;
  }

  switch (input) {
    case "/lookup": {
      const response = await postJSON(`${state.apiBase}/api/lookup`, {});
      shell.appendLog("lookup", JSON.stringify(response, null, 2));
      return;
    }
    case "/connect": {
      const response = await postJSON(`${state.apiBase}/api/connect`, {});
      shell.appendLog("connect", JSON.stringify(response, null, 2));
      return;
    }
    case "/inbox": {
      const lines = state.inbox.map((entry) => `[${entry.payload.from_peer_id}] ${entry.payload.message}`);
      shell.appendLog("inbox", lines.length === 0 ? "Inbox is empty." : lines.join("\n"));
      return;
    }
    case "/clear": {
      shell.clearLog();
      return;
    }
    case "/help": {
      shell.appendLog(
        "help",
        [
          "/lookup   resolve the current target peer",
          "/connect  test direct or relay connection setup",
          "/inbox    print received chat messages",
          "/clear    clear the terminal history",
          "/help     show this help text",
          "message   send chat text to the selected peer"
        ].join("\n")
      );
      return;
    }
    default: {
      shell.appendLog("error", `Unknown command: ${input}`);
    }
  }
}

async function postJSON(url, body) {
  const response = await fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json"
    },
    body: JSON.stringify(body)
  }).catch((error) => {
    throw new Error(`POST ${url} failed: ${error.message}`);
  });

  const payload = await response.json();
  if (!response.ok) {
    throw new Error(payload.error ?? `HTTP ${response.status}`);
  }
  return payload;
}

function describeError(context, error) {
  return `${context}: ${error.message}`;
}
