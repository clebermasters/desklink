package com.desklink.android

import android.content.Context
import android.net.ConnectivityManager
import java.net.Inet4Address

object Network {
    /** The phone's IPv4 address on its active network, which computers use to reach it. */
    fun wifiAddress(context: Context): String? {
        val connectivity = context.getSystemService(ConnectivityManager::class.java) ?: return null
        val network = connectivity.activeNetwork ?: return null
        val links = connectivity.getLinkProperties(network) ?: return null
        return links.linkAddresses
            .map { it.address }
            .firstOrNull { it is Inet4Address && !it.isLoopbackAddress && !it.isLinkLocalAddress }
            ?.hostAddress
    }
}
