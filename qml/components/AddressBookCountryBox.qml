// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Settings → Privacy: the country of address book numbers written without a country code. The core
// knows the countries, their calling codes and what Automatic stands for; here are their names.
// Made once the core has answered: Silica's ComboBox resets its choice when its items change.
Loader {
    id: box

    width: parent.width
    active: options.length > 0

    // The user chose another country.
    signal picked()

    // "" is Automatic.
    property var regions: [""]
    property var options: []
    property int choice: 0

    readonly property var names: ({
        //% "United Arab Emirates"
        "AE": qsTrId("threesea-la-country-ae"),
        //% "Austria"
        "AT": qsTrId("threesea-la-country-at"),
        //% "Australia"
        "AU": qsTrId("threesea-la-country-au"),
        //% "Belgium"
        "BE": qsTrId("threesea-la-country-be"),
        //% "Bulgaria"
        "BG": qsTrId("threesea-la-country-bg"),
        //% "Brazil"
        "BR": qsTrId("threesea-la-country-br"),
        //% "Canada"
        "CA": qsTrId("threesea-la-country-ca"),
        //% "Switzerland"
        "CH": qsTrId("threesea-la-country-ch"),
        //% "China"
        "CN": qsTrId("threesea-la-country-cn"),
        //% "Cyprus"
        "CY": qsTrId("threesea-la-country-cy"),
        //% "Czechia"
        "CZ": qsTrId("threesea-la-country-cz"),
        //% "Germany"
        "DE": qsTrId("threesea-la-country-de"),
        //% "Denmark"
        "DK": qsTrId("threesea-la-country-dk"),
        //% "Estonia"
        "EE": qsTrId("threesea-la-country-ee"),
        //% "Spain"
        "ES": qsTrId("threesea-la-country-es"),
        //% "Finland"
        "FI": qsTrId("threesea-la-country-fi"),
        //% "France"
        "FR": qsTrId("threesea-la-country-fr"),
        //% "United Kingdom"
        "GB": qsTrId("threesea-la-country-gb"),
        //% "Greece"
        "GR": qsTrId("threesea-la-country-gr"),
        //% "Croatia"
        "HR": qsTrId("threesea-la-country-hr"),
        //% "Hungary"
        "HU": qsTrId("threesea-la-country-hu"),
        //% "Ireland"
        "IE": qsTrId("threesea-la-country-ie"),
        //% "Israel"
        "IL": qsTrId("threesea-la-country-il"),
        //% "India"
        "IN": qsTrId("threesea-la-country-in"),
        //% "Iceland"
        "IS": qsTrId("threesea-la-country-is"),
        //% "Italy"
        "IT": qsTrId("threesea-la-country-it"),
        //% "Japan"
        "JP": qsTrId("threesea-la-country-jp"),
        //% "South Korea"
        "KR": qsTrId("threesea-la-country-kr"),
        //% "Liechtenstein"
        "LI": qsTrId("threesea-la-country-li"),
        //% "Luxembourg"
        "LU": qsTrId("threesea-la-country-lu"),
        //% "Latvia"
        "LV": qsTrId("threesea-la-country-lv"),
        //% "Malta"
        "MT": qsTrId("threesea-la-country-mt"),
        //% "Mexico"
        "MX": qsTrId("threesea-la-country-mx"),
        //% "Netherlands"
        "NL": qsTrId("threesea-la-country-nl"),
        //% "Norway"
        "NO": qsTrId("threesea-la-country-no"),
        //% "New Zealand"
        "NZ": qsTrId("threesea-la-country-nz"),
        //% "Poland"
        "PL": qsTrId("threesea-la-country-pl"),
        //% "Portugal"
        "PT": qsTrId("threesea-la-country-pt"),
        //% "Romania"
        "RO": qsTrId("threesea-la-country-ro"),
        //% "Serbia"
        "RS": qsTrId("threesea-la-country-rs"),
        //% "Russia"
        "RU": qsTrId("threesea-la-country-ru"),
        //% "Sweden"
        "SE": qsTrId("threesea-la-country-se"),
        //% "Singapore"
        "SG": qsTrId("threesea-la-country-sg"),
        //% "Slovenia"
        "SI": qsTrId("threesea-la-country-si"),
        //% "Slovakia"
        "SK": qsTrId("threesea-la-country-sk"),
        //% "Turkey"
        "TR": qsTrId("threesea-la-country-tr"),
        //% "Ukraine"
        "UA": qsTrId("threesea-la-country-ua"),
        //% "United States"
        "US": qsTrId("threesea-la-country-us"),
        //% "South Africa"
        "ZA": qsTrId("threesea-la-country-za")
    })

    function show(result) {
        var countries = result.countries.map(function(country) {
            return { region: country.region, name: names[country.region] || country.region, code: country.code }
        })
        countries.sort(function(a, b) { return a.name.localeCompare(b.name) })
        //% "Automatic"
        var automatic = qsTrId("threesea-la-address-book-country-automatic")
        if (result.automaticCode.length > 0) {
            //% "Automatic (+%1)"
            automatic = qsTrId("threesea-la-address-book-country-automatic-code").arg(result.automaticCode)
        }
        regions = [""].concat(countries.map(function(country) { return country.region }))
        choice = Math.max(0, regions.indexOf(result.chosen))
        options = [automatic].concat(countries.map(function(country) {
            return country.name + " (+" + country.code + ")"
        }))
    }

    Component.onCompleted: backend.call("addressBookCountry", {}, function(ok, result) {
        if (ok) box.show(result)
    })

    sourceComponent: Component {
        ChoiceBox {
            //% "Numbers without a country code"
            label: qsTrId("threesea-la-address-book-country")
            //% "Address book numbers written without a country code count as from this country. Automatic goes by your linked phone number, else by the device's region. Numbers from other countries are found when written with + and their country code."
            description: qsTrId("threesea-la-address-book-country-description")
            options: box.options
            choice: box.choice
            onChosen: {
                if (index === box.choice) return
                backend.call("setAddressBookCountry", { country: box.regions[index] }, function(ok, result) {
                    if (!ok) return
                    box.choice = Math.max(0, box.regions.indexOf(result.chosen))
                    box.picked()
                })
            }
        }
    }
}
