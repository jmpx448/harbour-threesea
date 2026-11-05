// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "Errors.js" as Errors

// Forwards messages, or sends what was shared in, to one or several chats without a chat page: one
// thing at a time, so that what the user does meanwhile gets its turn in the core between them, and
// one job after the other. Up here, as the page that asked is gone before they are through: what
// couldn't be sent is said once at the end.
Item {
    id: sender

    property var jobs: []
    property var job: null

    // `caption`: instead of what the messages have, empty for none, undefined to keep it.
    function forward(messages, identities, caption, hidePrivateChats) {
        var steps = []
        identities.forEach(function(identity) {
            messages.forEach(function(message) {
                steps.push({ identity: identity, message: message })
            })
        })
        add({ steps: steps, caption: caption, hidePrivateChats: hidePrivateChats, targets: identities.length })
    }

    // `text` goes as the first file's caption or, without files, as a message.
    function share(files, text, identities, hidePrivateChats) {
        var steps = []
        identities.forEach(function(identity) {
            if (files.length === 0) steps.push({ identity: identity, text: text })
            files.forEach(function(file, index) {
                steps.push({ identity: identity, file: file, caption: index === 0 ? text : "" })
            })
        })
        add({ steps: steps, shared: true, hidePrivateChats: hidePrivateChats, targets: identities.length })
    }

    function add(newJob) {
        newJob.at = 0
        newJob.kinds = []
        newJob.texts = {}
        newJob.failed = {}
        jobs = jobs.concat([newJob])
        if (!job) nextJob()
    }

    function nextJob() {
        job = jobs.length > 0 ? jobs[0] : null
        jobs = jobs.slice(1)
        if (job) nextStep()
    }

    function nextStep() {
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
        var failed = function(kind, message) {
            job.failed[step.identity] = true
            // A file of this phone can go as a file; a forwarded one has to be saved first.
            var shown = job.shared && kind === "pictureKeepsMetadata" ? "fileKeepsMetadata" : kind
            if (job.kinds.indexOf(shown) < 0) {
                job.kinds.push(shown)
                job.texts[shown] = message
            }
        }
        var answer = function(ok, result, kind, message) {
            if (!ok) failed(kind, message)
            done()
        }
        if (step.message !== undefined) {
            var args = { messages: [step.message], identity: step.identity }
            if (job.caption !== undefined) args.caption = job.caption
            backend.call("forwardMessages", args, function(ok, result, kind, message) {
                if (ok) result.errors.forEach(function(error) { failed(error) })
                else failed(kind, message)
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
        nextStep()
    }

    function finishJob() {
        frames.active = false
        for (var i = 0; i < job.kinds.length; ++i) {
            Notices.show(Errors.text(job.kinds[i], job.texts[job.kinds[i]]), Notice.Long, Notice.Center)
        }
        if (job.targets > 1) {
            var failed = Object.keys(job.failed).length
            Notices.show(failed > 0
                         //% "Not everything went to %n chat(s)"
                         ? qsTrId("threesea-la-not-sent-to-chats", failed)
                         //% "On its way to %n chat(s)"
                         : qsTrId("threesea-la-sent-to-chats", job.targets),
                         Notice.Long, Notice.Center)
        }
        // A private chat was unlocked only for this. Not if the user is in one by now.
        if (job.hidePrivateChats && !pageStack.find(function(page) {
            return page.objectName === "private" || page.isPrivate === true
        })) {
            backend.lockWithFilesOnTheirWay()
        }
        nextJob()
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
