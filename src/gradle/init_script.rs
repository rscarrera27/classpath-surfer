/// Gradle Groovy init script that registers a `classpathSurferExport` task in every
/// JVM subproject (those applying the `java`, `java-library`, or `kotlin("jvm")` plugin).
///
/// The task resolves the requested configurations, collects binary and source JAR
/// paths along with their Maven coordinates, and writes per-module JSON manifests
/// under `build/classpath-surfer/`.
pub const INIT_SCRIPT: &str = r#"
allprojects { project ->
    project.afterEvaluate {
        def hasJvmPlugin = project.plugins.hasPlugin('java') ||
            project.plugins.hasPlugin('java-library') ||
            project.plugins.hasPlugin('org.jetbrains.kotlin.jvm')

        if (hasJvmPlugin) {
            project.tasks.register('classpathSurferExport') {
                description = 'Exports resolved classpath for classpath-surfer tool'
                group = 'classpath-surfer'

                doLast {
                    def outputDir = new File(project.rootDir, "build/classpath-surfer")
                    outputDir.mkdirs()

                    def moduleName = project.path.replace(':', '_')
                    if (moduleName.isEmpty()) moduleName = '_root'
                    def outputFile = new File(outputDir, "classpath-${moduleName}.json")

                    def configNames = (project.findProperty('classpathSurfer.configurations')
                        ?: 'compileClasspath,runtimeClasspath').split(',')

                    def configs = []

                    configNames.each { configName ->
                        configName = configName.trim()
                        def config = project.configurations.findByName(configName)
                        if (config == null || !config.isCanBeResolved()) return

                        def deps = []
                        def binaryArtifacts = [:]

                        try {
                            config.incoming.artifacts.artifacts.each { artifact ->
                                def componentId = artifact.id.componentIdentifier
                                if (componentId instanceof org.gradle.api.artifacts.component.ModuleComponentIdentifier) {
                                    def key = "${componentId.group}:${componentId.module}:${componentId.version}"
                                    binaryArtifacts[key] = [
                                        group: componentId.group,
                                        artifact: componentId.module,
                                        version: componentId.version,
                                        jar_path: artifact.file.absolutePath
                                    ]
                                }
                            }
                        } catch (Exception e) {
                            project.logger.warn("classpath-surfer: Could not resolve ${configName}: ${e.message}")
                            return
                        }

                        // Resolve source JARs via detached configuration (best-effort)
                        def sourceFiles = [:]
                        try {
                            def sourceConfig = project.configurations.detachedConfiguration()
                            binaryArtifacts.each { key, info ->
                                sourceConfig.dependencies.add(
                                    project.dependencies.create("${info.group}:${info.artifact}:${info.version}:sources")
                                )
                            }
                            sourceConfig.transitive = false
                            sourceConfig.resolvedConfiguration.lenientConfiguration
                                .artifacts.each { artifact ->
                                    def compId = artifact.moduleVersion.id
                                    def key = "${compId.group}:${compId.name}:${compId.version}"
                                    sourceFiles[key] = artifact.file.absolutePath
                                }
                        } catch (Exception e) {
                            project.logger.info("classpath-surfer: Could not resolve some source JARs: ${e.message}")
                        }

                        // Merge binary + source info
                        binaryArtifacts.each { key, info ->
                            info['source_jar_path'] = sourceFiles[key]
                            info['classpath'] = configName
                            deps.add(info)
                        }

                        configs.add([
                            name: configName,
                            dependencies: deps
                        ])
                    }

                    def manifest = [
                        module_path: project.path,
                        configurations: configs
                    ]

                    outputFile.text = groovy.json.JsonOutput.prettyPrint(
                        groovy.json.JsonOutput.toJson(manifest)
                    )
                    project.logger.lifecycle("classpath-surfer: Wrote ${outputFile}")
                }
            }
        }
    }
}
"#;
