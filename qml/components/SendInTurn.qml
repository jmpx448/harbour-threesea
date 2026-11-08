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
// A forwarded picture or video whose metadata can't be removed is asked about at the end, one
// message at a time, if the window is there: accepted, it goes again as a plain file, as it is
// (ADR 0025); otherwise the notice says why it didn't go.
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
    // those of `identities` that the PIN has just shown. `keep`: the user's answer about metadata
    // that can't be removed.
    function forward(messages, identities, caption, hidden, keep) {
        var steps = []
        ordered(identities, hidden).forEach(function(identity) {
            messages.forEach(function(message) {
                steps.push({ identity: identity, message: message })
            })
        })
        add({ steps: steps, caption: caption, targets: identities.length, hideAfter: hidden.length * messages.length,
              keep: keep === true, hiddenIds: hidden.slice() })
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
        // Forwarded files refused for their metadata: [{message, name, video, identity}].
        newJob.unclean = []
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
            if (job.keep) args.keepMetadata = true
            backend.call("forwardMessages", args, function(ok, result, kind, message) {
                if (mine !== turn) return
                if (!ok) {
                    failed(kind, message)
                } else {
                    var unclean = result.keepsMetadata || []
                    unclean.forEach(function(file) {
                        job.unclean.push({ message: file.message, name: file.name, video: file.video,
                                           identity: step.identity })
                    })
                    // Those are asked about at the end, or said then.
                    var errors = result.errors || []
                    errors.forEach(function(error) {
                        if (unclean.length === 0 || (error !== "videoKeepsMetadata" && error !== "pictureKeepsMetadata")) {
                            failed(error)
                        }
                    })
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
            // A file to such a chat waits for a question at the end: the chat stays shown for its
            // answer, as long as the window is there (see onStateChanged below).
            var hidden = job.hiddenIds || []
            var asked = job.unclean.some(function(file) { return hidden.indexOf(file.identity) >= 0 })
            if (asked && Qt.application.state === Qt.ApplicationActive) job.hideOwed = true
            else hidePrivateChats()
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
        if (job && job.hideAfter > 0) return
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
        if (job.unclean.length > 0 && Qt.application.state === Qt.ApplicationActive) {
            if (job.hideOwed) hideOwed = true
            addQuestions(job.unclean, job.caption, job.hiddenIds || [])
        } else {
            if (job.hideOwed) hidePrivateChats()
            job.unclean.forEach(function(file) {
                job.failed[file.identity] = true
                var kind = file.video ? "videoKeepsMetadata" : "pictureKeepsMetadata"
                if (job.kinds.indexOf(kind) < 0) job.kinds.push(kind)
            })
        }
        for (var i = 0; i < job.kinds.length; ++i) {
            say(Errors.text(job.kinds[i], job.texts[job.kinds[i]]))
        }
        var failed = Object.keys(job.failed).length
        if (failed > 0 && (job.targets > 1 || job.kinds.length === 0)) {
            //% "Not everything went to %n chat(s)"
            say(qsTrId("threesea-la-not-sent-to-chats", failed))
        } else if (failed === 0 && job.unclean.length === 0 && job.targets > 1
                   && Qt.application.state === Qt.ApplicationActive) {
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

    // The forwarded files to ask about, one message at a time, each for all the chats it didn't
    // reach, those the PIN showed among them (`hidden`): they stay shown until the last answer
    // (`hideOwed`), and the answer goes to them as the first time did. Asked once nothing else
    // moves on the page stack.
    property var questions: []
    property bool asking
    property bool hideOwed
    function addQuestions(unclean, caption, hidden) {
        var list = questions.slice()
        unclean.forEach(function(file) {
            var same = list.filter(function(question) {
                return question.message === file.message && question.caption === caption
            })[0]
            if (!same) {
                same = { message: file.message, name: file.name, video: file.video,
                         identities: [], hidden: [], caption: caption }
                list.push(same)
            }
            if (same.identities.indexOf(file.identity) < 0) same.identities.push(file.identity)
            if (hidden.indexOf(file.identity) >= 0 && same.hidden.indexOf(file.identity) < 0) {
                same.hidden.push(file.identity)
            }
        })
        questions = list
        ask()
    }
    // No question left that needs the private chats shown: they are hidden again, or by the job
    // the last answer started.
    function settleHide() {
        if (!hideOwed || questions.some(function(question) { return question.hidden.length > 0 })) return
        hideOwed = false
        hidePrivateChats()
    }
    // The window went: what was for the private chats isn't asked any more, and they are hidden.
    function dropHiddenQuestions() {
        if (!hideOwed) return
        questions.forEach(function(question) {
            question.identities = question.identities.filter(function(identity) {
                return question.hidden.indexOf(identity) < 0
            })
            question.hidden = []
        })
        questions = questions.filter(function(question) { return question.identities.length > 0 })
        settleHide()
    }
    function ask() {
        if (asking || questions.length === 0 || pageStack.busy) return
        // Not over the lock, which takes the page stack away anyway (and is left with a pop).
        if (Qt.application.state !== Qt.ApplicationActive || window.locked || backend.state !== "ready") return
        var question = questions[0]
        var answered = false
        function answer(send) {
            if (answered) return
            answered = true
            asking = false
            questions = questions.filter(function(other) { return other !== question })
            if (send && question.identities.length > 0) {
                forward([question.message], question.identities, question.caption, question.hidden, true)
            }
            settleHide()
            ask()
        }
        asking = true
        var dialog = pageStack.push(Qt.resolvedUrl("../pages/VideoMetadataDialog.qml"),
                                    { name: question.name, video: question.video })
        dialog.accepted.connect(function() { answer(true) })
        dialog.rejected.connect(function() { answer(false) })
        // Taken off the stack by something else (the app lock): no answer, so not sent.
        dialog.done.connect(function() { if (dialog.result === DialogResult.None) answer(false) })
    }
    Connections {
        target: pageStack
        onBusyChanged: sender.ask()
    }
    Connections {
        target: Qt.application
        onStateChanged: {
            if (Qt.application.state !== Qt.ApplicationActive) sender.dropHiddenQuestions()
            sender.ask()
        }
    }

    // Everything forgotten: the data is gone, and with it the core that would have answered.
    function clear() {
        questions = []
        hideOwed = false
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
            var hide = (sender.job && sender.job.hideAfter > 0) || sender.hideOwed
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
