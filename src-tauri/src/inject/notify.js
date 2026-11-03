// Web notifications to Windows notifications. Runs at document start in the
// main frame of every service. Pages get a stand-in Notification that hands
// the title and text to the app's "notify" command, the only command a
// service page may call; the app decides whether to show it (muted accounts,
// Do Not Disturb, app lock). Clicking it opens the account and fires the
// notification's click, so the page opens that chat.
(() => {
  if (window.top !== window || window.__mcNotify) return;
  window.__mcNotify = true;
  const send = (args) => window.__TAURI_INTERNALS__?.invoke("notify", args).catch(() => {});
  const live = new Map();
  let next = 1;

  class Notification extends EventTarget {
    constructor(title, options = {}) {
      super();
      this.title = String(title ?? "");
      this.body = options.body == null ? "" : String(options.body);
      this.tag = options.tag == null ? "" : String(options.tag);
      this.data = options.data ?? null;
      this.icon = options.icon ?? "";
      this.silent = Boolean(options.silent);
      this.onclick = this.onclose = this.onshow = this.onerror = null;
      const id = next++;
      live.set(id, this);
      if (live.size > 50) live.delete(live.keys().next().value);
      send({ id, title: this.title.slice(0, 200), body: this.body.slice(0, 500) });
      setTimeout(() => this._fire("show"));
    }
    _fire(type) {
      const event = new Event(type, { cancelable: true });
      this.dispatchEvent(event);
      this[`on${type}`]?.call(this, event);
      return event;
    }
    close() {
      this._fire("close");
    }
    static get permission() {
      return "granted";
    }
    static requestPermission(callback) {
      callback?.("granted");
      return Promise.resolve("granted");
    }
    static get maxActions() {
      return 0;
    }
  }
  Object.defineProperty(window, "Notification", { value: Notification, writable: true, configurable: true });

  // Pages that show notifications through their service worker's registration.
  if (window.ServiceWorkerRegistration) {
    ServiceWorkerRegistration.prototype.showNotification = function (title, options) {
      new Notification(title, options);
      return Promise.resolve();
    };
    ServiceWorkerRegistration.prototype.getNotifications = () => Promise.resolve([]);
  }
  const query = navigator.permissions?.query?.bind(navigator.permissions);
  if (query) {
    navigator.permissions.query = (descriptor) =>
      descriptor?.name === "notifications"
        ? Promise.resolve(Object.assign(new EventTarget(), { state: "granted", status: "granted", name: "notifications", onchange: null }))
        : query(descriptor);
  }

  // Called by the app when a notification is clicked.
  window.__mcNotificationClicked = (id) => {
    const notification = live.get(id);
    if (!notification) return;
    window.focus();
    notification._fire("click");
  };
})();
