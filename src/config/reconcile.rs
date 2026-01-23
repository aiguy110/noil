use crate::config::diff::create_diff;
use crate::config::parse::ConfigError;
use crate::config::version::compute_config_hash;
use crate::storage::traits::{ConfigSource, ConfigState, ConfigVersion, Storage};
use chrono::Utc;
use console::style;
use dialoguer::{Confirm, Input};
use similar::TextDiff;
use std::path::Path;

/// Result of config reconciliation
#[derive(Debug)]
pub enum ReconcileResult {
    /// Config was initialized in the database for the first time
    Initialized { hash: String },
    /// No changes detected
    NoChange,
    /// File was fast-forwarded to match DB
    FastForwardedFile { from_hash: String, to_hash: String },
    /// DB was fast-forwarded to match file
    FastForwardedDB { from_hash: String, to_hash: String },
    /// Configs were merged successfully
    Merged {
        base_hash: String,
        file_hash: String,
        db_hash: String,
        merged_hash: String,
    },
    /// Unresolved conflict (should have exited already)
    UnresolvedConflict { conflict_file: String },
}

/// Main reconciliation function called on startup
pub async fn reconcile_config_on_startup(
    config_path: &Path,
    storage: &dyn Storage,
) -> Result<ReconcileResult, ConfigError> {
    // 1. Check for unresolved conflicts
    if let Some(state) = storage.get_config_state().await? {
        if state.has_conflict {
            return handle_existing_conflict(config_path, storage, &state).await;
        }
    }

    // 2. Read file and compute hash
    let file_content = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(e) => {
            return Err(ConfigError::Io(e));
        }
    };
    let file_hash = compute_config_hash(&file_content);

    // 3. Get active DB version
    let db_version = storage.get_active_config_version().await?;

    // 4. Handle cases
    match db_version {
        None => {
            // No active version - check if this version exists but is inactive
            println!("{}", style("Initializing config in database...").cyan());

            // Check if version already exists (e.g., from previous UI save that was never hot-reloaded)
            let existing_version = storage.get_config_version(&file_hash).await?;

            if existing_version.is_some() {
                // Version exists but is inactive - just mark it active
                storage.mark_config_active(&file_hash).await?;
            } else {
                // Version doesn't exist - insert it
                let version = ConfigVersion {
                    version_hash: file_hash.clone(),
                    parent_hash: None,
                    yaml_content: file_content,
                    created_at: Utc::now(),
                    source: ConfigSource::File,
                    is_active: true,
                };
                storage.insert_config_version(&version).await?;
            }

            // Update state
            let state = ConfigState {
                has_conflict: false,
                conflict_file_path: None,
                file_version_hash: Some(file_hash.clone()),
                db_version_hash: Some(file_hash.clone()),
            };
            storage.update_config_state(&state).await?;

            Ok(ReconcileResult::Initialized { hash: file_hash })
        }
        Some(db_version) => {
            if file_hash == db_version.version_hash {
                // No change
                Ok(ReconcileResult::NoChange)
            } else {
                // Changes detected - determine relationship
                reconcile_diverged_configs(config_path, storage, &file_content, &file_hash, &db_version).await
            }
        }
    }
}

/// Handle case where there's an existing conflict
async fn handle_existing_conflict(
    config_path: &Path,
    storage: &dyn Storage,
    state: &ConfigState,
) -> Result<ReconcileResult, ConfigError> {
    println!("{}", style("⚠ Unresolved config conflict detected!").red().bold());

    let conflict_file = state
        .conflict_file_path
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or_else(|| config_path.to_str().unwrap_or("config file"));

    println!("Conflict markers were written to: {}", style(conflict_file).yellow());
    println!();

    // Check if file still has conflict markers
    let file_content = std::fs::read_to_string(config_path)
        .map_err(|e| ConfigError::Io(e))?;

    if has_conflict_markers(&file_content) {
        println!("The file still contains conflict markers (<<<<<<<, =======, >>>>>>>).");
        println!("Please resolve the conflicts manually and restart.");
        return Err(ConfigError::Validation(
            "Unresolved conflict markers in config file".to_string(),
        ));
    }

    // Conflict appears resolved, ask user to confirm
    let resolved = Confirm::new()
        .with_prompt("Have you resolved all conflicts?")
        .default(false)
        .interact()
        .map_err(|e| ConfigError::Validation(format!("Failed to get user input: {}", e)))?;

    if !resolved {
        return Err(ConfigError::Validation(
            "User indicated conflicts not resolved".to_string(),
        ));
    }

    // Validate the resolved content
    let file_hash = compute_config_hash(&file_content);
    match serde_yaml::from_str::<serde_yaml::Value>(&file_content) {
        Ok(_) => {
            println!("{}", style("✓ Config file is valid YAML").green());

            // Check if this version already exists
            let existing_version = storage.get_config_version(&file_hash).await?;

            if existing_version.is_some() {
                // Version exists but is inactive - just mark it active
                storage.mark_config_active(&file_hash).await?;
            } else {
                // Store as new version
                let version = ConfigVersion {
                    version_hash: file_hash.clone(),
                    parent_hash: state.db_version_hash.clone(),
                    yaml_content: file_content,
                    created_at: Utc::now(),
                    source: ConfigSource::Merge,
                    is_active: true,
                };
                storage.insert_config_version(&version).await?;
            }

            // Clear conflict state
            let new_state = ConfigState {
                has_conflict: false,
                conflict_file_path: None,
                file_version_hash: Some(file_hash.clone()),
                db_version_hash: Some(file_hash.clone()),
            };
            storage.update_config_state(&new_state).await?;

            println!("{}", style("✓ Conflict resolved and config updated").green());
            Ok(ReconcileResult::Merged {
                base_hash: state.file_version_hash.clone().unwrap_or_default(),
                file_hash: file_hash.clone(),
                db_hash: state.db_version_hash.clone().unwrap_or_default(),
                merged_hash: file_hash,
            })
        }
        Err(e) => {
            println!("{}", style(format!("✗ Invalid YAML: {}", e)).red());
            Err(ConfigError::Validation(format!(
                "Resolved config is not valid YAML: {}",
                e
            )))
        }
    }
}

/// Check if content has git-style conflict markers
fn has_conflict_markers(content: &str) -> bool {
    content.contains("<<<<<<<") && content.contains("=======") && content.contains(">>>>>>>")
}

/// Reconcile when file and DB have diverged
async fn reconcile_diverged_configs(
    config_path: &Path,
    storage: &dyn Storage,
    file_content: &str,
    file_hash: &str,
    db_version: &ConfigVersion,
) -> Result<ReconcileResult, ConfigError> {
    // Get state to determine last known hashes
    let state = storage.get_config_state().await?;

    // Check if we can fast-forward
    if let Some(state) = state.as_ref() {
        // Case 1: DB ahead, file matches last known file hash
        if Some(file_hash.to_string()) == state.file_version_hash
            && Some(db_version.version_hash.clone()) != state.db_version_hash
        {
            println!(
                "{}",
                style("Database config has been updated since last run").yellow()
            );
            println!("File: {}", style(file_hash).cyan());
            println!("DB:   {}", style(&db_version.version_hash).green());
            println!();

            let update = Confirm::new()
                .with_prompt("Update config file to match database?")
                .default(false)
                .interact()
                .map_err(|e| ConfigError::Validation(format!("Failed to get user input: {}", e)))?;

            if update {
                std::fs::write(config_path, &db_version.yaml_content)
                    .map_err(|e| ConfigError::Io(e))?;

                // Update state
                let new_state = ConfigState {
                    has_conflict: false,
                    conflict_file_path: None,
                    file_version_hash: Some(db_version.version_hash.clone()),
                    db_version_hash: Some(db_version.version_hash.clone()),
                };
                storage.update_config_state(&new_state).await?;

                println!("{}", style("✓ Config file updated from database").green());
                return Ok(ReconcileResult::FastForwardedFile {
                    from_hash: file_hash.to_string(),
                    to_hash: db_version.version_hash.clone(),
                });
            } else {
                println!("{}", style("Config file not updated. Using current file.").yellow());
                // Continue to merge
            }
        }

        // Case 2: File ahead, DB matches last known DB hash
        if Some(db_version.version_hash.clone()) == state.db_version_hash
            && Some(file_hash.to_string()) != state.file_version_hash
        {
            println!(
                "{}",
                style("Config file has been updated since last run").yellow()
            );
            println!("File: {}", style(file_hash).green());
            println!("DB:   {}", style(&db_version.version_hash).cyan());
            println!();

            // Show diff
            let diff = create_diff(&db_version.yaml_content, file_content);
            println!("Changes in file:");
            println!("{}", style(&diff).dim());
            println!();

            let update = Confirm::new()
                .with_prompt("Update database to match config file?")
                .default(true)
                .interact()
                .map_err(|e| ConfigError::Validation(format!("Failed to get user input: {}", e)))?;

            if update {
                // Check if this version already exists (e.g., from previous UI save)
                let existing_version = storage.get_config_version(file_hash).await?;

                if existing_version.is_some() {
                    // Version exists but is inactive - just mark it active
                    storage.mark_config_active(file_hash).await?;
                } else {
                    // Version doesn't exist - insert it
                    let version = ConfigVersion {
                        version_hash: file_hash.to_string(),
                        parent_hash: Some(db_version.version_hash.clone()),
                        yaml_content: file_content.to_string(),
                        created_at: Utc::now(),
                        source: ConfigSource::File,
                        is_active: true,
                    };
                    storage.insert_config_version(&version).await?;
                }

                // Update state
                let new_state = ConfigState {
                    has_conflict: false,
                    conflict_file_path: None,
                    file_version_hash: Some(file_hash.to_string()),
                    db_version_hash: Some(file_hash.to_string()),
                };
                storage.update_config_state(&new_state).await?;

                println!("{}", style("✓ Database updated from config file").green());
                return Ok(ReconcileResult::FastForwardedDB {
                    from_hash: db_version.version_hash.clone(),
                    to_hash: file_hash.to_string(),
                });
            }
        }
    }

    // Case 3: Both changed (diverged) - need 3-way merge
    println!("{}", style("Config has diverged - both file and database were modified").yellow().bold());
    println!("File: {}", style(file_hash).cyan());
    println!("DB:   {}", style(&db_version.version_hash).cyan());
    println!();

    // Find common ancestor
    let base_version = if let Some(parent_hash) = &db_version.parent_hash {
        storage.get_config_version(parent_hash).await?
    } else {
        None
    };

    perform_three_way_merge(
        config_path,
        storage,
        base_version.as_ref(),
        db_version,
        file_content,
        file_hash,
    )
    .await
}

/// Perform 3-way merge
async fn perform_three_way_merge(
    config_path: &Path,
    storage: &dyn Storage,
    base_version: Option<&ConfigVersion>,
    db_version: &ConfigVersion,
    file_content: &str,
    file_hash: &str,
) -> Result<ReconcileResult, ConfigError> {
    let base_content = base_version
        .map(|v| v.yaml_content.as_str())
        .unwrap_or("");

    println!("Attempting 3-way merge...");
    println!();

    // Perform merge
    let merge_result = merge_three_way(base_content, &db_version.yaml_content, file_content);

    match merge_result {
        MergeResult::Clean(merged) => {
            // Validate merged YAML
            match serde_yaml::from_str::<serde_yaml::Value>(&merged) {
                Ok(_) => {
                    println!("{}", style("✓ Merge completed successfully").green());
                    println!();

                    // Show diff
                    let diff = create_diff(&db_version.yaml_content, &merged);
                    if !diff.trim().is_empty() {
                        println!("Merged changes:");
                        println!("{}", style(&diff).dim());
                        println!();
                    }

                    let accept = Confirm::new()
                        .with_prompt("Accept merged configuration?")
                        .default(true)
                        .interact()
                        .map_err(|e| ConfigError::Validation(format!("Failed to get user input: {}", e)))?;

                    if !accept {
                        return Err(ConfigError::Validation("User rejected merge".to_string()));
                    }

                    // Ask if user wants to write back to file
                    let write_file = Confirm::new()
                        .with_prompt("Write merged config back to file?")
                        .default(true)
                        .interact()
                        .map_err(|e| ConfigError::Validation(format!("Failed to get user input: {}", e)))?;

                    if write_file {
                        std::fs::write(config_path, &merged).map_err(|e| ConfigError::Io(e))?;
                        println!("{}", style("✓ Config file updated").green());
                    }

                    // Save merged version
                    let merged_hash = compute_config_hash(&merged);

                    // Check if this version already exists
                    let existing_version = storage.get_config_version(&merged_hash).await?;

                    if existing_version.is_some() {
                        // Version exists but is inactive - just mark it active
                        storage.mark_config_active(&merged_hash).await?;
                    } else {
                        // Store as new version
                        let version = ConfigVersion {
                            version_hash: merged_hash.clone(),
                            parent_hash: Some(db_version.version_hash.clone()),
                            yaml_content: merged,
                            created_at: Utc::now(),
                            source: ConfigSource::Merge,
                            is_active: true,
                        };
                        storage.insert_config_version(&version).await?;
                    }

                    // Update state
                    let state = ConfigState {
                        has_conflict: false,
                        conflict_file_path: None,
                        file_version_hash: Some(merged_hash.clone()),
                        db_version_hash: Some(merged_hash.clone()),
                    };
                    storage.update_config_state(&state).await?;

                    println!("{}", style("✓ Merged config saved to database").green());

                    Ok(ReconcileResult::Merged {
                        base_hash: base_version
                            .map(|v| v.version_hash.clone())
                            .unwrap_or_default(),
                        file_hash: file_hash.to_string(),
                        db_hash: db_version.version_hash.clone(),
                        merged_hash,
                    })
                }
                Err(e) => {
                    println!("{}", style(format!("✗ Merged result is not valid YAML: {}", e)).red());
                    Err(ConfigError::Validation(format!(
                        "Merge produced invalid YAML: {}",
                        e
                    )))
                }
            }
        }
        MergeResult::Conflict(with_markers) => {
            println!("{}", style("✗ Merge conflicts detected").red().bold());
            println!();

            // Ask where to write conflict file
            let default_path = config_path.to_str().unwrap_or("config.yml");
            let conflict_path: String = Input::new()
                .with_prompt("Write conflicts to")
                .default(default_path.to_string())
                .interact_text()
                .map_err(|e| ConfigError::Validation(format!("Failed to get user input: {}", e)))?;

            // Write conflict markers
            std::fs::write(&conflict_path, &with_markers).map_err(|e| ConfigError::Io(e))?;

            println!(
                "{}",
                style(format!("Conflict markers written to: {}", conflict_path))
                    .yellow()
            );
            println!("Please resolve conflicts and restart the application.");
            println!();
            println!("Conflict markers:");
            println!("  <<<<<<< FILE (your changes)");
            println!("  ======= (divider)");
            println!("  >>>>>>> DB (database version)");

            // Set conflict state
            let state = ConfigState {
                has_conflict: true,
                conflict_file_path: Some(conflict_path.clone()),
                file_version_hash: Some(file_hash.to_string()),
                db_version_hash: Some(db_version.version_hash.clone()),
            };
            storage.update_config_state(&state).await?;

            Err(ConfigError::Validation(format!(
                "Unresolved merge conflicts in {}",
                conflict_path
            )))
        }
    }
}

/// Result of a 3-way merge
enum MergeResult {
    /// Clean merge with no conflicts
    Clean(String),
    /// Merge with conflicts (contains conflict markers)
    Conflict(String),
}

/// Simple 3-way merge implementation
fn merge_three_way(base: &str, ours: &str, theirs: &str) -> MergeResult {
    let base_lines: Vec<&str> = base.lines().collect();
    let ours_lines: Vec<&str> = ours.lines().collect();
    let theirs_lines: Vec<&str> = theirs.lines().collect();

    // Compute diffs from base (for future improvements)
    let _diff_ours = TextDiff::from_slices(&base_lines, &ours_lines);
    let _diff_theirs = TextDiff::from_slices(&base_lines, &theirs_lines);

    let mut result = Vec::new();
    let mut has_conflict = false;

    let mut base_idx = 0;
    let mut ours_idx = 0;
    let mut theirs_idx = 0;

    // This is a simplified merge algorithm
    // For production use, a more sophisticated algorithm would be better
    while base_idx < base_lines.len()
        || ours_idx < ours_lines.len()
        || theirs_idx < theirs_lines.len()
    {
        let base_line = base_lines.get(base_idx).copied();
        let ours_line = ours_lines.get(ours_idx).copied();
        let theirs_line = theirs_lines.get(theirs_idx).copied();

        if base_line == ours_line && base_line == theirs_line {
            // No changes
            if let Some(line) = base_line {
                result.push(line.to_string());
            }
            base_idx += 1;
            ours_idx += 1;
            theirs_idx += 1;
        } else if ours_line == theirs_line {
            // Both made same change
            if let Some(line) = ours_line {
                result.push(line.to_string());
            }
            base_idx += 1;
            ours_idx += 1;
            theirs_idx += 1;
        } else if base_line == ours_line && base_line != theirs_line {
            // Only theirs changed, accept theirs
            if let Some(line) = theirs_line {
                result.push(line.to_string());
            }
            base_idx += 1;
            ours_idx += 1;
            theirs_idx += 1;
        } else if base_line == theirs_line && base_line != ours_line {
            // Only ours changed, accept ours
            if let Some(line) = ours_line {
                result.push(line.to_string());
            }
            base_idx += 1;
            ours_idx += 1;
            theirs_idx += 1;
        } else {
            // Conflict
            has_conflict = true;
            result.push("<<<<<<< FILE".to_string());
            if let Some(line) = theirs_line {
                result.push(line.to_string());
            }
            result.push("=======".to_string());
            if let Some(line) = ours_line {
                result.push(line.to_string());
            }
            result.push(">>>>>>> DB".to_string());

            base_idx += 1;
            ours_idx += 1;
            theirs_idx += 1;
        }
    }

    let merged = result.join("\n");
    if has_conflict {
        MergeResult::Conflict(merged)
    } else {
        MergeResult::Clean(merged)
    }
}
