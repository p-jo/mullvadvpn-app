package net.mullvad.mullvadvpn.ui.customdns

import android.view.View
import android.widget.TextView
import java.net.InetAddress
import kotlin.properties.Delegates.observable
import net.mullvad.mullvadvpn.R

class EditCustomDnsServerHolder(view: View, adapter: CustomDnsAdapter) : CustomDnsItemHolder(view) {
    private val input: TextView = view.findViewById(R.id.input)

    var serverAddress by observable<InetAddress?>(null) { _, _, hostNameAndAddressText ->
        if (hostNameAndAddressText != null) {
            val hostNameAndAddress = hostNameAndAddressText.toString().split('/', limit = 2)
            val address = hostNameAndAddress[1]

            input.text = address
        } else {
            input.text = ""
        }
    }

    init {
        view.findViewById<View>(R.id.save).setOnClickListener {
            adapter.saveDnsServer(input.text.toString())
        }
    }
}
