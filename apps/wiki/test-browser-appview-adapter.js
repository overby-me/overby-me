// test-browser.nu asks the backend what a click made of it, in GraphQL, from
// inside the page. On the AppView (`--appview`) this stands where that `gql()`
// stood and answers the same few documents from XRPC, in Hasura's shape, so
// that one suite holds both backends to the same checks. `__API__` is filled in
// by the suite. Synchronous on purpose, as the original is: WebDriver's
// execute/sync returns what the script returns.
var __s;
try {
	__s = JSON.parse(localStorage.getItem("wiki_session"));
} catch (e) {}
var __T = __s ? __s.access_token : "";

function xrpc(method, params, body) {
	var qs = params
		? "?" +
			Object.keys(params)
				.map((k) => k + "=" + encodeURIComponent(params[k]))
				.join("&")
		: "";
	var x = new XMLHttpRequest();
	x.open(
		body === undefined ? "GET" : "POST",
		"__API__/xrpc/wiki.radikal." + method + qs,
		false,
	);
	x.setRequestHeader("authorization", "Bearer " + __T);
	if (body !== undefined)
		x.setRequestHeader("content-type", "application/json");
	try {
		x.send(body === undefined ? null : JSON.stringify(body));
	} catch (e) {
		return null;
	}
	if (x.status < 200 || x.status >= 300) return null;
	try {
		return JSON.parse(x.responseText);
	} catch (e) {
		return {};
	}
}

var KIND = {
	"wiki/folder": "folder",
	"wiki/document": "document",
	"wiki/file": "file",
	"vote/policy": "policy",
	"vote/position": "position",
	"vote/candidate": "candidate",
	"vote/change": "change",
	"vote/question": "question",
	"vote/poll": "poll",
	"wiki/group": "group",
	"wiki/event": "event",
};

// A node as a Hasura row: a page's body lives in `data.content` there, and a
// context is its own context.
function rowOf(n) {
	var data = {};
	if (n.data && typeof n.data === "object")
		for (var k in n.data) data[k] = n.data[k];
	if (n.content !== undefined && n.content !== null) data.content = n.content;
	return {
		id: n.id,
		key: n.slug,
		name: n.title !== undefined ? n.title : n.name,
		contextId: n.context_id || n.id,
		parentId: n.parent_id || null,
		kind: n.kind,
		data: data,
	};
}

function childrenOf(parent) {
	var r = xrpc("listChildren", { parent: parent });
	return r ? (r.children || []).map(rowOf) : [];
}

// A ballot is no node here. What the check wants of one is how many there are,
// and what the caller's own says. A secret ballot says nothing of whose it is,
// to the server either, so its choices are read off the count, which is that
// ballot and nothing else while it is the only one.
function ballotsOf(poll) {
	var p = xrpc("getPoll", { id: poll });
	if (!p) return [];
	var own = p.viewer && p.viewer.choices;
	if (!own && p.ballots === 1 && p.counts) {
		own = [];
		p.counts.forEach((n, option) => {
			for (var k = 0; k < n; k++) own.push(option);
		});
	}
	var rows = [];
	for (var i = 0; i < p.ballots; i++)
		rows.push({ id: poll + "#" + i, data: i === 0 && own ? own : null });
	return rows;
}

// The contexts the caller sits in without owning them.
function seatsNotOwned() {
	var mine = xrpc("listContexts", { scope: "mine" });
	var rows = [];
	((mine && mine.contexts) || []).forEach((c) => {
		var seen = xrpc("getNode", { id: c.id });
		if (seen && seen.viewer && !seen.viewer.is_context_owner) {
			rows.push({
				parent: {
					key: c.slug,
					mimeId: "wiki/" + c.kind,
					parentId: c.parent_id || null,
				},
			});
		}
	});
	return rows;
}

function gql(q, v) {
	v = v || {};
	var mime = (q.match(/mimeId:\{_eq:"([^"]+)"\}/) || [])[1];
	if (q.indexOf("deleteNode") >= 0) {
		var gone =
			xrpc("deleteDocument", null, { id: v.i }) ||
			xrpc("deleteContext", null, { id: v.i });
		return { data: { deleteNode: gone ? { id: v.i } : null } };
	}
	// Rules are code here, not rows: there is nothing of the kind to delete.
	if (q.indexOf("deletePermissions") >= 0)
		return { data: { deletePermissions: { affected_rows: 0 } } };
	if (q.indexOf("deleteMembers") >= 0)
		return { data: { deleteMembers: { affected_rows: 0 } } };
	if (q.indexOf("insertMembers") >= 0) {
		var put = 0;
		(v.o || []).forEach((m) => {
			// Whoever makes a context owns it already: the owner row is the interim's.
			if (m.nodeId) {
				put++;
				return;
			}
			var r = xrpc("inviteMembers", null, {
				context_id: m.parentId,
				invites: [{ name: m.name, email: m.email }],
			});
			if (r) put += r.inserted;
		});
		return { data: { insertMembers: { affected_rows: put } } };
	}
	if (q.indexOf("members(where") >= 0)
		return { data: { members: seatsNotOwned() } };
	if (q.indexOf("node(id:$i)") >= 0) {
		var one = xrpc("getNode", { id: v.i });
		return { data: { node: one && one.node ? rowOf(one.node) : null } };
	}
	if (mime === "wiki/home") {
		var home = xrpc("getNode", { path: "" });
		return { data: { nodes: home && home.node ? [rowOf(home.node)] : [] } };
	}
	if (q.indexOf("parentId:{_eq:$p}") >= 0) {
		var rows;
		if (mime === "vote/vote") rows = ballotsOf(v.p);
		else if (mime === "vote/comment")
			rows = ((xrpc("getComments", { on: v.p }) || {}).comments || []).map(
				(c) => ({ id: c.id }),
			);
		else if (mime === "speak/list")
			rows = (
				(xrpc("listSpeakerLists", { context: v.p }) || {}).lists || []
			).map((l) => ({ id: l.id }));
		else if (mime === "vote/poll")
			rows = ((xrpc("listPolls", { parent: v.p }) || {}).polls || []).map(
				(p) => ({ id: p.id }),
			);
		else rows = childrenOf(v.p).filter((n) => !mime || n.kind === KIND[mime]);
		if (v.n !== undefined) rows = rows.filter((n) => n.name === v.n);
		return { data: { nodes: rows } };
	}
	if (q.indexOf("name:{_eq:$n}") >= 0) {
		// A group made from the home is at the top of the tree; an event is made
		// inside a group. Whoever made either has a seat in it.
		var seen = {};
		var made = ["roots", "mine"]
			.reduce(
				(all, scope) =>
					all.concat(
						((xrpc("listContexts", { scope: scope }) || {}).contexts || []).map(
							rowOf,
						),
					),
				[],
			)
			.filter((n) => {
				var first = !seen[n.id];
				seen[n.id] = true;
				return first && n.name === v.n && n.kind === KIND[mime];
			});
		return { data: { nodes: made } };
	}
	return {
		errors: [{ message: "the AppView adapter has no answer for: " + q }],
	};
}
