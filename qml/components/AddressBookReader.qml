// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import org.nemomobile.contacts 1.0

// Reads the device's address book (names, e-mail addresses, phone numbers) for matching.
// Loaded through a Loader so a device without the contacts module just can't sync.
Item {
    id: reader

    // [{firstName, lastName, emails, phones}] once read.
    signal finished(var entries)

    property bool started: false

    function start() {
        started = true
        if (people.populated) collect()
    }

    function collect() {
        var entries = []
        for (var i = 0; i < people.count; ++i) {
            var person = people.personByRow(i)
            if (!person) continue
            person.ensureComplete()
            var emails = []
            var phones = []
            var e = person.emailDetails || []
            for (var j = 0; j < e.length; ++j) if (e[j].address) emails.push(e[j].address)
            var p = person.phoneDetails || []
            for (var k = 0; k < p.length; ++k) if (p[k].number) phones.push(p[k].number)
            if (emails.length > 0 || phones.length > 0) {
                entries.push({ firstName: person.firstName || "", lastName: person.lastName || "",
                               emails: emails, phones: phones })
            }
        }
        started = false
        finished(entries)
    }

    PeopleModel {
        id: people
        filterType: PeopleModel.FilterAll
        requiredProperty: PeopleModel.PhoneNumberRequired | PeopleModel.EmailAddressRequired
        onPopulatedChanged: if (populated && reader.started) reader.collect()
    }
}
