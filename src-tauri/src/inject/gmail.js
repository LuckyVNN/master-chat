// New mail, described. Gmail only notifies if you turn its desktop
// notifications on, and then without much detail, so this watches for unread
// mail instead: when a new email arrives, it raises a notification
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

  // Mark all as read (the account's right-click menu), the way you'd do it by
  // hand: search the inbox for unread mail, select all of it, Mark as read,
  // confirm, back to the inbox. Gmail's own ids (gh="mtb" for the toolbar,
  // act="1" for Mark as read) don't depend on its language.
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const shown = (el) => el && el.getBoundingClientRect().width > 0;
  const find = (selector) => [...document.querySelectorAll(selector)].find(shown);
  const until = async (selector, ms) => {
    for (let t = 0; t < ms; t += 250) {
      const el = find(selector);
      if (el) return el;
      await sleep(250);
    }
    return null;
  };
  const press = (el) => {
    for (const type of ["mousedown", "mouseup"]) el.dispatchEvent(new MouseEvent(type, { bubbles: true }));
    el.click();
  };
  let marking = false;
  window.__mcMarkAllRead = async () => {
    if (marking) return;
    marking = true;
    try {
      location.hash = "#search/in%3Ainbox+is%3Aunread";
      await sleep(2500);
      if (!find("tr.zA.zE")) return;
      const all = await until('[gh="mtb"] [role="checkbox"]', 8000);
      if (!all) return;
      press(all);
      const markRead = await until('[gh="mtb"] [act="1"]', 4000);
      if (!markRead) return;
      // More than a page: "Select all conversations that match this search".
      const everything = await until(".ya span.x8", 1500);
      if (everything) {
        press(everything);
        await sleep(600);
      }
      press(markRead);
      // Gmail asks to confirm an action on many conversations.
      const ok = await until('button[name="ok"]', 3000);
      if (ok) press(ok);
      await sleep(2500);
    } finally {
      marking = false;
      location.hash = "#inbox";
    }
  };

  // New mail comes from Gmail's own feed of unread inbox mail: it lists every
  // message (a reply in a thread you've seen too), whatever page or inbox tab
  // you're on. Each one is told once, by its id; what's unread when the app
  // starts is old news.
  const base = () => (location.pathname.match(/^\/mail\/u\/\d+/) || ["/mail/u/0"])[0];
  const told = new Set();
  let primed = false;
  let feedWorks = false;
  // Promotions and Social mail isn't news: their own feeds say which to skip.
  const SKIP = ["^sq_ig_i_promo", "^sq_ig_i_social"];
  const feed = async (label = "") => {
    const path = label ? `/feed/atom/${encodeURIComponent(label)}` : "/feed/atom";
    const response = await fetch(`${base()}${path}`, { credentials: "include", cache: "no-store" });
    if (!response.ok) return null;
    const xml = new DOMParser().parseFromString(await response.text(), "application/xml");
    return xml.querySelector("parsererror") ? null : xml;
  };
  const idOf = (entry) => (entry.getElementsByTagName("id")[0]?.textContent || "").trim();
  const pollFeed = async () => {
    try {
      const [xml, ...skipped] = await Promise.all([feed(), ...SKIP.map((label) => feed(label).catch(() => null))]);
      if (!xml) return;
      feedWorks = true;
      const skip = new Set(skipped.flatMap((x) => (x ? [...x.getElementsByTagName("entry")].map(idOf) : [])));
      for (const entry of xml.getElementsByTagName("entry")) {
        const get = (tag, from = entry) => (from.getElementsByTagName(tag)[0]?.textContent || "").trim();
        const id = idOf(entry);
        if (!id || told.has(id)) continue;
        told.add(id);
        if (!primed || skip.has(id)) continue;
        const author = entry.getElementsByTagName("author")[0];
        const name = (author && (get("name", author) || get("email", author))) || "New email";
        const subject = get("title");
        const preview = get("summary");
        const link = entry.getElementsByTagName("link")[0]?.getAttribute("href") || "";
        const messageId = link.match(/message_id=([0-9a-f]+)/i)?.[1];
        const notification = new Notification(name, { body: [subject, preview].filter(Boolean).join(" — ") });
        notification.onclick = () => {
          if (messageId) location.hash = `#inbox/${messageId}`;
        };
      }
      primed = true;
    } catch {
      // Offline or signed out: try again next time.
    }
  };
  pollFeed();
  setInterval(pollFeed, 20000);

  // If the feed can't be read, the inbox list on screen is watched instead.
  const notified = new Set();
  let quietUntil = 0;
  setInterval(() => {
    if (feedWorks || !inInbox()) return;
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
