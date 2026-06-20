package org.mochios.kome.lsp

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServerSupportProvider

@Suppress("DEPRECATION")
class KomeLspServerSupportProvider :
    LspServerSupportProvider {

    override fun fileOpened(
        project: Project,
        file: VirtualFile,
        serverStarter: LspServerSupportProvider.LspServerStarter,
    ) {
        LOG.warn(
            "Kome LSP provider: opened ${file.path}, " +
                    "extension=${file.extension}",
        )

        if (file.extension?.lowercase() != "kome") {
            return
        }

        LOG.warn(
            "Kome LSP provider: starting server for ${file.path}",
        )

        serverStarter.ensureServerStarted(
            KomeLspServerDescriptor(project),
        )
    }

    companion object {
        private val LOG = Logger.getInstance(
            KomeLspServerSupportProvider::class.java,
        )
    }
}