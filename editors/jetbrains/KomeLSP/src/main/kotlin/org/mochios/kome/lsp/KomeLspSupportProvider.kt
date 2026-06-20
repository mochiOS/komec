package org.mochios.kome.lsp

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor

@Suppress("DEPRECATION")
internal class KomeLspServerDescriptor(
    project: Project,
) : ProjectWideLspServerDescriptor(
    project,
    "Kome Language Server",
) {
    override fun isSupportedFile(
        file: VirtualFile,
    ): Boolean {
        return file.extension == "kome"
    }

    override fun createCommandLine(): GeneralCommandLine {
        return GeneralCommandLine(
            KomeLspExecutable.resolve(),
        ).apply {
            project.basePath?.let(::withWorkDirectory)
        }
    }

    override fun getLanguageId(
        file: VirtualFile,
    ): String {
        return "kome"
    }
}