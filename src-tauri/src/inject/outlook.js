// Outlook's unread count. Outlook doesn't put it in the page title, so this
// reads the number beside Inbox in the folder list and makes the title
// "(N) Outlook", which the app reads like any other page title.
(() => {
  if (window.top !== window || window.__mcOutlook) return;
  window.__mcOutlook = true;

  // Inbox, in the languages Outlook is most used in here; data-folder-name is
  // English whatever the language, where Outlook sets it.
  const INBOX = ["inbox", "hộp thư đến", "posteingang", "boîte de réception", "bandeja de entrada", "posta in arrivo", "caixa de entrada", "受信トレイ", "收件箱", "받은 편지함"];
  const isInbox = (el) => {
    if (el.getAttribute("data-folder-name")?.toLowerCase() === "inbox") return true;
    const name = (el.getAttribute("title") || el.getAttribute("aria-label") || "").toLowerCase();
    return INBOX.some((inbox) => name.startsWith(inbox));
  };
  const inboxUnread = () => {
    const node = [...document.querySelectorAll('[data-folder-name], [role="treeitem"]')].find(isInbox);
    if (!node) return 0;
    // The count is a span of its own holding just the number.
    const counts = [...node.querySelectorAll("span")].map((s) => s.textContent.trim()).filter((t) => /^\d+$/.test(t));
    return counts.length ? Number(counts[counts.length - 1]) : 0;
  };

  const titleProp = Object.getOwnPropertyDescriptor(Document.prototype, "title");
  let unread = 0;
  let pageTitle = "Outlook";
  const applyTitle = () => {
    const wanted = unread ? `(${unread}) ${pageTitle}` : pageTitle;
    if (titleProp.get.call(document) !== wanted) titleProp.set.call(document, wanted);
  };
  Object.defineProperty(document, "title", {
    configurable: true,
    get: () => titleProp.get.call(document),
    set: (value) => {
      pageTitle = String(value).replace(/^\(\d+\)\s*/, "") || "Outlook";
      applyTitle();
    },
  });
  setInterval(() => {
    unread = inboxUnread();
    applyTitle();
  }, 2000);
})();
