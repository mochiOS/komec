package org.mochios.kome.lsp

import com.intellij.execution.ExecutionException
import com.intellij.execution.configurations.PathEnvironmentVariableUtil
import com.intellij.openapi.util.SystemInfo
import java.io.File

internal object KomeLspExecutable {
    private const val ENVIRONMENT_VARIABLE = "KOME_LSP_PATH"

    fun resolve(): String {
        val configuredPath = System.getenv(ENVIRONMENT_VARIABLE)
            ?.trim()
            ?.takeIf { it.isNotEmpty() }

        if (configuredPath != null) {
            val executable = File(configuredPath)

            if (!executable.isFile) {
                throw ExecutionException(
                    "$ENVIRONMENT_VARIABLE points to a missing file: " +
                            executable.absolutePath,
                )
            }

            return executable.absolutePath
        }

        val executableName = if (SystemInfo.isWindows) {
            "kome-lsp.exe"
        } else {
            "kome-lsp"
        }

        val executable = PathEnvironmentVariableUtil.findInPath(
            executableName,
            PathEnvironmentVariableUtil.getPathVariableValue(),
            null,
        )

        if (executable != null) {
            return executable.absolutePath
        }

        throw ExecutionException(
            "Kome Language Server was not found. " +
                    "Set KOME_LSP_PATH or add $executableName to PATH.",
        )
    }
}