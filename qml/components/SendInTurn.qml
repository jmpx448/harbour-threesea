// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "Errors.js" as Errors

// Forwards messages, or sends what was shared in, to one or several chats without a chat page: one
// thing at a time, so that what the user does meanwhile gets its turn in the core between them, and
// one job after the other. Up here, as the page that asked is gone before they are through: what
// couldn't be sent is said once at the end. Closing the window doesn't end the process meanwhile
// (setSendingInTurn), up to a limit; only a package update's kill or the like still can, and then it
// says so itself.
//
// Private chats the PIN showed for a job are readable in the core until it is hidden again, so
// they come first: the job goes ahead of those waiting, at the next step of the running one, and
// `lockWithFilesOnTheirWay` follows their last step, not the end.
Item {
    id: sender

    property var jobs: []
    property var job: null
    // Each answer belongs to one step: one that comes after the job was given up finds another turn.
    property int turn: 0

    // Hidden ones first.
    function ordered(identities, hidden) {
        return hidden.concat(identities.filter(function(identity) { return hidden.indexOf(identity) < 0 }))
    }

    // `caption`: instead of what the messages have, empty for none, undefined to keep it. `hidden`:
    // those of `identities` that the PIN has just shown.
    function forward(messages, identities, caption, hidden) {
        var steps = []
        ordered(identities, hidden).forEach(function(identity) {
            messages.forEach(function(message) {
                steps.push({ identity: identity, message: message })
            })
        })
        add({ steps: steps, caption: caption, targets: identities.length, hideAfter: hidden.length * messages.length })
    }

    // `text` goes as the first file's caption or, without files, as a message.
    function share(files, text, identities, hidden) {
        var steps = []
        ordered(identities, hidden).forEach(function(identity) {
            if (files.length === 0) steps.push({ identity: identity, text: text })
            files.forEach(function(file, index) {
                steps.push({ identity: identity, file: file, caption: index === 0 ? text : "" })
            })
        })
        add({ steps: steps, shared: true, targets: identities.length,
              hideAfter: hidden.length * Math.max(files.length, 1) })
    }

    function add(newJob) {
        newJob.at = 0
        newJob.kinds = []
        newJob.texts = {}
        newJob.failed = {}
        // Others with chats the PIN showed go first, in their order.
        var ahead = 0
        if (newJob.hideAfter > 0) {
            while (ahead < jobs.length && jobs[ahead].hideAfter > 0) ++ahead
        } else {
            ahead = jobs.length
        }
        jobs = jobs.slice(0, ahead).concat([newJob]).concat(jobs.slice(ahead))
        if (!job) {
            backend.setSendingInTurn(true)
            nextJob()
        }
    }

    function nextJob() {
        job = jobs.length > 0 ? jobs[0] : null
        jobs = jobs.slice(1)
        if (job) {
            nextStep()
        } else {
            limit.stop()
            backend.setSendingInTurn(false)
            window.sentInTurn()
        }
    }

    function nextStep() {
        limit.restart()
        if (job.at >= job.steps.length) {
            finishJob()
            return
        }
        var step = job.steps[job.at]
        if (step.file && step.file.video && !step.file.read) {
            frames.active = true
            frames.item.start(step.file)
            return
        }
        send(step)
    }

    function send(step) {
        var mine = ++turn
        var failed = function(kind, message) {
            job.failed[step.identity] = true
            if (!kind) return
            // A file of this phone can go as a file; a forwarded one has to be saved first.
            var shown = job.shared && kind === "pictureKeepsMetadata" ? "fileKeepsMetadata" : kind
            if (job.kinds.indexOf(shown) < 0) {
                job.kinds.push(shown)
                job.texts[shown] = message
            }
        }
        var answer = function(ok, result, kind, message) {
            if (mine !== turn) return
            if (!ok) failed(kind, message)
            else if (result && result.failedMembers > 0) failed()
            done()
        }
        if (step.message !== undefined) {
            var args = { messages: [step.message], identity: step.identity }
            if (job.caption !== undefined) args.caption = job.caption
            backend.call("forwardMessages", args, function(ok, result, kind, message) {
                if (mine !== turn) return
                if (!ok) {
                    failed(kind, message)
                } else {
                    (result.errors || []).forEach(function(error) { failed(error) })
                    if (result.failedMembers > 0) failed()
                }
                done()
            })
        } else if (step.file && step.file.video) {
            backend.sendVideo(step.identity, step.file.path, step.file.frame, step.file.seconds || 0, step.caption, answer)
        } else if (step.file) {
            backend.sendFile(step.identity, step.file.path, step.file.photo, step.caption, answer)
        } else {
            backend.call("sendText", { identity: step.identity, text: step.text }, answer)
        }
    }

    function done() {
        ++job.at
        if (job.at === job.hideAfter) {
            job.hideAfter = 0
            hidePrivateChats()
        }
        // One waiting with chats the PIN showed: this one goes on after it.
        if (jobs.length > 0 && jobs[0].hideAfter > 0 && !(job.hideAfter > 0)) {
            jobs = [jobs[0], job].concat(jobs.slice(1))
            nextJob()
            return
        }
        nextStep()
    }

    // Not while another job still has chats to reach that way, whose end does it; nor if the user
    // has gone into one by now.
    function hidePrivateChats() {
        if (jobs.length > 0 && jobs[0].hideAfter > 0) return
        if (pageStack.find(function(page) { return page.objectName === "private" || page.isPrivate === true })) return
        backend.lockWithFilesOnTheirWay()
    }

    // In the window if it is there to be seen, else as a system notification. Fixed texts, without
    // any chat's name.
    function say(text) {
        if (Qt.application.state === Qt.ApplicationActive) {
            Notices.show(text, Notice.Long, Notice.Center)
        } else {
            //% "Threesea did not send everything"
            window.plainNotification("sendInTurn", qsTrId("threesea-la-send-in-turn-failed"), text)
        }
    }

    function finishJob() {
        frames.active = false
        for (var i = 0; i < job.kinds.length; ++i) {
            say(Errors.text(job.kinds[i], job.texts[job.kinds[i]]))
        }
        var failed = Object.keys(job.failed).length
        if (failed > 0 && (job.targets > 1 || job.kinds.length === 0)) {
            //% "Not everything went to %n chat(s)"
            say(qsTrId("threesea-la-not-sent-to-chats", failed))
        } else if (failed === 0 && job.targets > 1 && Qt.application.state === Qt.ApplicationActive) {
            //% "On its way to %n chat(s)"
            say(qsTrId("threesea-la-sent-to-chats", job.targets))
        }
        nextJob()
    }

    // Nothing can be asked of what isn't going any further.
    function sayWhatIsLost() {
        if (!job) return
        window.plainNotification("sendInTurn", qsTrId("threesea-la-send-in-turn-failed"),
                                 //% "Some chats did not get what you sent them. Send it to them again."
                                 qsTrId("threesea-la-send-in-turn-lost"))
    }

    // Everything forgotten: the data is gone, and with it the core that would have answered.
    function clear() {
        ++turn
        limit.stop()
        frames.active = false
        jobs = []
        job = null
        backend.setSendingInTurn(false)
    }

    // A step that never gets an answer, or takes longer than scaling a video can: what waits behind
    // it isn't held up for ever, nor is a process nobody wants.
    Timer {
        id: limit
        interval: 900000
        onTriggered: {
            sender.sayWhatIsLost()
            var hide = sender.job && sender.job.hideAfter > 0
            sender.clear()
            if (hide) backend.lockWithFilesOnTheirWay()
            window.sentInTurn()
        }
    }

    Loader {
        id: frames
        active: false
        sourceComponent: VideoFrameReader {
            onFinished: {
                entry.read = true
                sender.nextStep()
            }
        }
    }
}
