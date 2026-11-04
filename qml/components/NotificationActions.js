// SPDX-License-Identifier: AGPL-3.0-only
.pragma library

// Reply and Mark as read from a notification, on their way to the core. The core doesn't exist yet
// when such a tap is what starts the app (the key comes from Sailfish Secrets first), nor until the
// passphrase has opened the stored data, and the bridge drops whatever it is given without a core.
// So they wait here, in memory. Lipstick has already closed the notification by then, and the
// action's token is used up, so this is the only place that still has a typed reply, except for a
// copy in its chat's draft wherever a core runs to take it (`kept`: the window, for the app lock).

var waiting = []

// {identity, text}, or {identity} to mark the chat as read only.
function add(action) {
    waiting.push(action)
}

// The actions the core can take in this state of the backend, oldest first; they leave this list.
// In any state with a core it answers for itself, with an error if it can't do what is asked.
function release(state) {
    if (state === "starting" || state === "locked" || state === "removing" || state === "error") {
        return []
    }
    var actions = waiting
    waiting = []
    return actions
}

function replyWaiting() {
    return waiting.some(function(action) { return action.text !== undefined })
}

// The replies still without a copy in their chat's draft.
function repliesToKeep() {
    return waiting.filter(function(action) { return action.text !== undefined && !action.kept && !action.keeping })
}

// Whether each reply has that copy.
function allRepliesKept() {
    return waiting.every(function(action) { return action.text === undefined || action.kept === true })
}

// The core won't start in this run. Says whether that lost a reply.
function drop() {
    var lostReply = replyWaiting()
    waiting = []
    return lostReply
}
