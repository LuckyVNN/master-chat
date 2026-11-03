// New mail, described. Gmail only notifies if you turn its desktop
// notifications on, and then without much detail, so this watches the inbox
// list instead: when an unread email appears, it raises a notification
// (through notify.js) with the sender, subject and preview. Clicking it opens
// the email.
(() => {
  if (window.top !== window || window.__mcGmail) return;
  window.__mcGmail = true;

  const inInbox = () => /^#(inbox|$)/.test(location.hash || "#");
  const text = (el) => (el?.textContent || "").replace(/\s+/g, " ").trim();
  // A row: tr.zA, unread with .zE. The sender's element carries its address and
  // name, the subject is .bog and the preview .y2 (" - preview").
  const describe = (row) => {
    const from = row.querySelector("[email]");
    const name = from?.getAttribute("name") || text(from) || from?.getAttribute("email") || "";
    const subject = text(row.querySelector(".bog"));
    const preview = text(row.querySelector(".y2")).replace(/^[-–]\s*/, "");
    return { name, subject, preview, key: `${from?.getAttribute("email") || name}|${subject}` };
  };

  const notified = new Set();
  let quietUntil = 0;
  setInterval(() => {
    if (!inInbox()) return;
    const rows = [...document.querySelectorAll("tr.zA.zE")];
    if (!document.querySelector("tr.zA")) return;
    // The list fills in as Gmail loads; what's unread by then is old news.
    if (!quietUntil) quietUntil = Date.now() + 10000;
    for (const row of rows) {
      const { name, subject, preview, key } = describe(row);
      if (!name || notified.has(key)) continue;
      notified.add(key);
      if (Date.now() < quietUntil) continue;
      const notification = new Notification(name, { body: [subject, preview].filter(Boolean).join(" — ") });
      notification.onclick = () => {
        const again = [...document.querySelectorAll("tr.zA")].find((r) => describe(r).key === key);
        again?.click();
      };
    }
  }, 2000);
})();
