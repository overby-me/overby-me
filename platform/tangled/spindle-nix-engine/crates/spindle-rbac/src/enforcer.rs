//! RBAC enforcer using `casbin-rs`, matching the upstream Go spindle's
//! authorization model.
//!
//! The upstream Go spindle uses a casbin RBAC model with the following
//! resource hierarchy:
//!
//! - **Spindle** — The spindle instance itself.
//!   - Roles: `owner`, `member`
//!   - Actions: `invite` (only owners can invite new members)
//! - **Repo** — A repository tracked by the spindle.
//!   - Roles: `owner`, `collaborator`
//!   - Actions: `invite` (only repo owners can invite collaborators)
//!
//! # Model
//!
//! Uses a RBAC model with resource types:
//! ```text
//! [request_definition]
//! r = sub, obj, act
//!
//! [policy_definition]
//! p = sub, obj, act
//!
//! [role_definition]
//! g = _, _, _
//!
//! [policy_effect]
//! e = some(where (p.eft == allow))
//!
//! [matchers]
//! m = g(r.sub, p.sub, r.obj) && r.obj == p.obj && r.act == p.act
//! ```
//!
//! The third parameter in `g` scopes roles to a specific resource, so a user
//! can be an `owner` of the spindle but only a `collaborator` on a specific repo.

use std::sync::Arc;

use casbin::prelude::*;
use tokio::sync::RwLock;

// Re-alias Result to std's version since casbin::prelude::* shadows it
type Result<T, E = RbacError> = std::result::Result<T, E>;

/// The casbin model definition for the spindle RBAC system.
///
/// Uses grouped (scoped) RBAC: `g(user, role, resource)` means "user has role
/// on resource". This allows the same user to have different roles on different
/// resources (e.g. owner of spindle, collaborator on a repo).
const MODEL_TEXT: &str = r#"
[request_definition]
r = sub, obj, act

[policy_definition]
p = sub, obj, act

[role_definition]
g = _, _, _

[policy_effect]
e = some(where (p.eft == allow))

[matchers]
m = g(r.sub, p.sub, r.obj) && r.obj == p.obj && r.act == p.act
"#;

/// Resource identifier for the spindle instance itself.
const SPINDLE_RESOURCE: &str = "spindle";

/// Errors that can occur during RBAC operations.
#[derive(Debug, thiserror::Error)]
pub enum RbacError {
    /// A casbin error occurred.
    #[error("casbin error: {0}")]
    Casbin(String),

    /// The enforcer has not been initialized.
    #[error("rbac enforcer not initialized")]
    NotInitialized,
}

impl From<casbin::Error> for RbacError {
    fn from(e: casbin::Error) -> Self {
        RbacError::Casbin(e.to_string())
    }
}

/// RBAC enforcer for the spindle.
///
/// Thread-safe wrapper around a `casbin::Enforcer` that provides
/// spindle-specific authorization operations matching the upstream Go
/// spindle's RBAC interface.
///
/// # Usage
///
/// ```no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use spindle_rbac::enforcer::SpindleEnforcer;
///
/// let enforcer = SpindleEnforcer::new().await?;
/// enforcer.add_spindle("did:web:spindle.example.com").await?;
/// enforcer.add_spindle_owner("did:web:spindle.example.com", "did:plc:owner123").await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct SpindleEnforcer {
    enforcer: Arc<RwLock<Enforcer>>,
}

impl std::fmt::Debug for SpindleEnforcer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpindleEnforcer").finish_non_exhaustive()
    }
}

impl SpindleEnforcer {
    /// Create a new RBAC enforcer with the spindle authorization model.
    ///
    /// The enforcer starts with an empty policy — call [`add_spindle`](Self::add_spindle)
    /// and [`add_spindle_owner`](Self::add_spindle_owner) to bootstrap.
    pub async fn new() -> Result<Self, RbacError> {
        let model = DefaultModel::from_str(MODEL_TEXT).await?;
        let enforcer = Enforcer::new(model, ()).await?;

        Ok(Self {
            enforcer: Arc::new(RwLock::new(enforcer)),
        })
    }

    // -----------------------------------------------------------------------
    // Spindle-level operations
    // -----------------------------------------------------------------------

    /// Register the spindle instance in the RBAC system.
    ///
    /// Adds policies that define what roles can do on the spindle resource:
    /// - `owner` can `invite` members
    /// - `member` has basic access (no special actions yet)
    ///
    /// Matches the upstream Go `AddSpindle` function.
    pub async fn add_spindle(&self, _spindle_did: &str) -> Result<(), RbacError> {
        let mut e = self.enforcer.write().await;

        // Policy: owners of the spindle can invite new members
        let policy = vec![
            "owner".to_string(),
            SPINDLE_RESOURCE.to_string(),
            "invite".to_string(),
        ];
        e.add_policy(policy).await.ok();

        Ok(())
    }

    /// Add a DID as the spindle owner.
    ///
    /// Assigns the `owner` role on the `spindle` resource to the given DID.
    ///
    /// Matches the upstream Go `AddSpindleOwner` function.
    pub async fn add_spindle_owner(
        &self,
        spindle_did: &str,
        owner_did: &str,
    ) -> Result<(), RbacError> {
        let mut e = self.enforcer.write().await;

        // g(owner_did, "owner", "spindle") — user has owner role on spindle
        let _ = spindle_did; // The spindle DID is used for context but the resource is always "spindle"
        e.add_named_grouping_policy(
            "g",
            vec![
                owner_did.to_string(),
                "owner".to_string(),
                SPINDLE_RESOURCE.to_string(),
            ],
        )
        .await
        .ok();

        Ok(())
    }

    /// Add a DID as a spindle member.
    ///
    /// Assigns the `member` role on the `spindle` resource to the given DID.
    ///
    /// Matches the upstream Go `AddSpindleMember` function.
    pub async fn add_spindle_member(&self, member_did: &str) -> Result<(), RbacError> {
        let mut e = self.enforcer.write().await;

        e.add_named_grouping_policy(
            "g",
            vec![
                member_did.to_string(),
                "member".to_string(),
                SPINDLE_RESOURCE.to_string(),
            ],
        )
        .await
        .ok();

        Ok(())
    }

    /// Remove a DID from spindle membership.
    ///
    /// Removes both `member` and `owner` roles on the spindle resource.
    ///
    /// Matches the upstream Go `RemoveSpindleMember` function.
    pub async fn remove_spindle_member(&self, member_did: &str) -> Result<(), RbacError> {
        let mut e = self.enforcer.write().await;

        // Try removing both roles (ok if they don't exist)
        e.remove_named_grouping_policy(
            "g",
            vec![
                member_did.to_string(),
                "member".to_string(),
                SPINDLE_RESOURCE.to_string(),
            ],
        )
        .await
        .ok();

        e.remove_named_grouping_policy(
            "g",
            vec![
                member_did.to_string(),
                "owner".to_string(),
                SPINDLE_RESOURCE.to_string(),
            ],
        )
        .await
        .ok();

        Ok(())
    }

    /// Check whether a DID is allowed to invite new spindle members.
    ///
    /// Only spindle owners can invite.
    ///
    /// Matches the upstream Go `IsSpindleInviteAllowed` function.
    pub async fn is_spindle_invite_allowed(&self, did: &str) -> Result<bool, RbacError> {
        let e = self.enforcer.read().await;
        let result = e.enforce((did, SPINDLE_RESOURCE, "invite"))?;
        Ok(result)
    }

    /// Get all DIDs that have a specific role on the spindle.
    ///
    /// Matches the upstream Go `GetSpindleUsersByRole` function.
    pub async fn get_spindle_users_by_role(&self, role: &str) -> Result<Vec<String>, RbacError> {
        let e = self.enforcer.read().await;

        // Get all grouping policies and filter for the spindle resource + requested role
        let policies = e.get_named_grouping_policy("g");
        let users: Vec<String> = policies
            .into_iter()
            .filter(|p| p.len() >= 3 && p[1] == role && p[2] == SPINDLE_RESOURCE)
            .map(|p| p[0].clone())
            .collect();

        Ok(users)
    }

    /// Check whether a DID has any role on the spindle (owner or member).
    pub async fn is_spindle_user(&self, did: &str) -> Result<bool, RbacError> {
        let e = self.enforcer.read().await;
        let roles = e.get_implicit_roles_for_user(did, Some(SPINDLE_RESOURCE));
        Ok(!roles.is_empty())
    }

    // -----------------------------------------------------------------------
    // Repo-level operations
    // -----------------------------------------------------------------------

    /// Build the resource identifier for a repository.
    ///
    /// Format: `repo:{did}/{name}` — this scopes roles to a specific repository.
    fn repo_resource(did: &str, name: &str) -> String {
        format!("repo:{did}/{name}")
    }

    /// Register a repository in the RBAC system.
    ///
    /// Adds policies defining what roles can do on this repo:
    /// - `owner` can `invite` collaborators
    ///
    /// Also assigns the repo owner DID as the `owner` role on this repo.
    ///
    /// Matches the upstream Go `AddRepo` function.
    pub async fn add_repo(&self, owner_did: &str, repo_name: &str) -> Result<(), RbacError> {
        let mut e = self.enforcer.write().await;
        let resource = Self::repo_resource(owner_did, repo_name);

        // Policy: owners of this repo can invite collaborators
        e.add_policy(vec![
            "owner".to_string(),
            resource.clone(),
            "invite".to_string(),
        ])
        .await
        .ok();

        // Assign the repo owner DID as owner of this repo resource
        e.add_named_grouping_policy(
            "g",
            vec![owner_did.to_string(), "owner".to_string(), resource],
        )
        .await
        .ok();

        Ok(())
    }

    /// Add a collaborator to a repository.
    ///
    /// Assigns the `collaborator` role on the repo resource to the given DID.
    ///
    /// Matches the upstream Go `AddCollaborator` function.
    pub async fn add_collaborator(
        &self,
        repo_owner_did: &str,
        repo_name: &str,
        collaborator_did: &str,
    ) -> Result<(), RbacError> {
        let mut e = self.enforcer.write().await;
        let resource = Self::repo_resource(repo_owner_did, repo_name);

        e.add_named_grouping_policy(
            "g",
            vec![
                collaborator_did.to_string(),
                "collaborator".to_string(),
                resource,
            ],
        )
        .await
        .ok();

        Ok(())
    }

    /// Remove a collaborator from a repository.
    pub async fn remove_collaborator(
        &self,
        repo_owner_did: &str,
        repo_name: &str,
        collaborator_did: &str,
    ) -> Result<(), RbacError> {
        let mut e = self.enforcer.write().await;
        let resource = Self::repo_resource(repo_owner_did, repo_name);

        e.remove_named_grouping_policy(
            "g",
            vec![
                collaborator_did.to_string(),
                "collaborator".to_string(),
                resource,
            ],
        )
        .await
        .ok();

        Ok(())
    }

    /// Check whether a DID is allowed to invite collaborators to a repo.
    ///
    /// Only repo owners can invite.
    ///
    /// Matches the upstream Go `IsCollaboratorInviteAllowed` function.
    pub async fn is_collaborator_invite_allowed(
        &self,
        did: &str,
        repo_owner_did: &str,
        repo_name: &str,
    ) -> Result<bool, RbacError> {
        let e = self.enforcer.read().await;
        let resource = Self::repo_resource(repo_owner_did, repo_name);
        let result = e.enforce((did, resource.as_str(), "invite"))?;
        Ok(result)
    }

    /// Check whether a DID has any role on a repo (owner or collaborator).
    pub async fn is_repo_user(
        &self,
        did: &str,
        repo_owner_did: &str,
        repo_name: &str,
    ) -> Result<bool, RbacError> {
        let e = self.enforcer.read().await;
        let resource = Self::repo_resource(repo_owner_did, repo_name);
        let roles = e.get_implicit_roles_for_user(did, Some(&resource));
        Ok(!roles.is_empty())
    }

    /// Get all users with a specific role on a repository.
    pub async fn get_repo_users_by_role(
        &self,
        repo_owner_did: &str,
        repo_name: &str,
        role: &str,
    ) -> Result<Vec<String>, RbacError> {
        let e = self.enforcer.read().await;
        let resource = Self::repo_resource(repo_owner_did, repo_name);

        let policies = e.get_named_grouping_policy("g");
        let users: Vec<String> = policies
            .into_iter()
            .filter(|p| p.len() >= 3 && p[1] == role && p[2] == resource)
            .map(|p| p[0].clone())
            .collect();

        Ok(users)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_enforcer() {
        let enforcer = SpindleEnforcer::new().await.unwrap();
        // Should be able to create without errors
        let _ = format!("{:?}", enforcer);
    }

    #[tokio::test]
    async fn spindle_owner_can_invite() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_spindle("did:web:spindle.example.com").await.unwrap();
        e.add_spindle_owner("did:web:spindle.example.com", "did:plc:owner123")
            .await
            .unwrap();

        assert!(
            e.is_spindle_invite_allowed("did:plc:owner123")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn spindle_member_cannot_invite() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_spindle("did:web:spindle.example.com").await.unwrap();
        e.add_spindle_member("did:plc:member456").await.unwrap();

        assert!(
            !e.is_spindle_invite_allowed("did:plc:member456")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn unknown_user_cannot_invite() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_spindle("did:web:spindle.example.com").await.unwrap();

        assert!(
            !e.is_spindle_invite_allowed("did:plc:stranger")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn add_and_remove_spindle_member() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_spindle("did:web:spindle.example.com").await.unwrap();
        e.add_spindle_member("did:plc:alice").await.unwrap();

        assert!(e.is_spindle_user("did:plc:alice").await.unwrap());

        e.remove_spindle_member("did:plc:alice").await.unwrap();

        assert!(!e.is_spindle_user("did:plc:alice").await.unwrap());
    }

    #[tokio::test]
    async fn get_spindle_users_by_role_owners() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_spindle("did:web:spindle.example.com").await.unwrap();
        e.add_spindle_owner("did:web:spindle.example.com", "did:plc:owner1")
            .await
            .unwrap();
        e.add_spindle_owner("did:web:spindle.example.com", "did:plc:owner2")
            .await
            .unwrap();
        e.add_spindle_member("did:plc:member1").await.unwrap();

        let owners = e.get_spindle_users_by_role("owner").await.unwrap();
        assert_eq!(owners.len(), 2);
        assert!(owners.contains(&"did:plc:owner1".to_string()));
        assert!(owners.contains(&"did:plc:owner2".to_string()));

        let members = e.get_spindle_users_by_role("member").await.unwrap();
        assert_eq!(members.len(), 1);
        assert!(members.contains(&"did:plc:member1".to_string()));
    }

    #[tokio::test]
    async fn repo_owner_can_invite_collaborators() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_repo("did:plc:alice", "my-repo").await.unwrap();

        assert!(
            e.is_collaborator_invite_allowed("did:plc:alice", "did:plc:alice", "my-repo")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn non_owner_cannot_invite_collaborators() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_repo("did:plc:alice", "my-repo").await.unwrap();

        assert!(
            !e.is_collaborator_invite_allowed("did:plc:bob", "did:plc:alice", "my-repo")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn add_and_check_collaborator() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_repo("did:plc:alice", "my-repo").await.unwrap();
        e.add_collaborator("did:plc:alice", "my-repo", "did:plc:bob")
            .await
            .unwrap();

        assert!(
            e.is_repo_user("did:plc:bob", "did:plc:alice", "my-repo")
                .await
                .unwrap()
        );
        assert!(
            e.is_repo_user("did:plc:alice", "did:plc:alice", "my-repo")
                .await
                .unwrap()
        );
        assert!(
            !e.is_repo_user("did:plc:charlie", "did:plc:alice", "my-repo")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn remove_collaborator() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_repo("did:plc:alice", "my-repo").await.unwrap();
        e.add_collaborator("did:plc:alice", "my-repo", "did:plc:bob")
            .await
            .unwrap();

        assert!(
            e.is_repo_user("did:plc:bob", "did:plc:alice", "my-repo")
                .await
                .unwrap()
        );

        e.remove_collaborator("did:plc:alice", "my-repo", "did:plc:bob")
            .await
            .unwrap();

        assert!(
            !e.is_repo_user("did:plc:bob", "did:plc:alice", "my-repo")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn get_repo_users_by_role_collaborators() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_repo("did:plc:alice", "my-repo").await.unwrap();
        e.add_collaborator("did:plc:alice", "my-repo", "did:plc:bob")
            .await
            .unwrap();
        e.add_collaborator("did:plc:alice", "my-repo", "did:plc:charlie")
            .await
            .unwrap();

        let collabs = e
            .get_repo_users_by_role("did:plc:alice", "my-repo", "collaborator")
            .await
            .unwrap();
        assert_eq!(collabs.len(), 2);
        assert!(collabs.contains(&"did:plc:bob".to_string()));
        assert!(collabs.contains(&"did:plc:charlie".to_string()));

        let owners = e
            .get_repo_users_by_role("did:plc:alice", "my-repo", "owner")
            .await
            .unwrap();
        assert_eq!(owners.len(), 1);
        assert!(owners.contains(&"did:plc:alice".to_string()));
    }

    #[tokio::test]
    async fn repo_roles_are_scoped() {
        let e = SpindleEnforcer::new().await.unwrap();

        // Alice owns repo-a, Bob owns repo-b
        e.add_repo("did:plc:alice", "repo-a").await.unwrap();
        e.add_repo("did:plc:bob", "repo-b").await.unwrap();

        // Alice should be owner of repo-a but not repo-b
        assert!(
            e.is_repo_user("did:plc:alice", "did:plc:alice", "repo-a")
                .await
                .unwrap()
        );
        assert!(
            !e.is_repo_user("did:plc:alice", "did:plc:bob", "repo-b")
                .await
                .unwrap()
        );

        // Bob should be owner of repo-b but not repo-a
        assert!(
            e.is_repo_user("did:plc:bob", "did:plc:bob", "repo-b")
                .await
                .unwrap()
        );
        assert!(
            !e.is_repo_user("did:plc:bob", "did:plc:alice", "repo-a")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn spindle_and_repo_roles_independent() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_spindle("did:web:spindle.example.com").await.unwrap();
        e.add_spindle_member("did:plc:alice").await.unwrap();
        e.add_repo("did:plc:bob", "my-repo").await.unwrap();

        // Alice is a spindle member but has no repo role
        assert!(e.is_spindle_user("did:plc:alice").await.unwrap());
        assert!(
            !e.is_repo_user("did:plc:alice", "did:plc:bob", "my-repo")
                .await
                .unwrap()
        );

        // Bob is a repo owner but not a spindle member
        assert!(!e.is_spindle_user("did:plc:bob").await.unwrap());
        assert!(
            e.is_repo_user("did:plc:bob", "did:plc:bob", "my-repo")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn remove_spindle_member_removes_owner_too() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_spindle("did:web:spindle.example.com").await.unwrap();
        e.add_spindle_owner("did:web:spindle.example.com", "did:plc:alice")
            .await
            .unwrap();

        assert!(e.is_spindle_user("did:plc:alice").await.unwrap());
        assert!(e.is_spindle_invite_allowed("did:plc:alice").await.unwrap());

        e.remove_spindle_member("did:plc:alice").await.unwrap();

        assert!(!e.is_spindle_user("did:plc:alice").await.unwrap());
        assert!(!e.is_spindle_invite_allowed("did:plc:alice").await.unwrap());
    }

    #[tokio::test]
    async fn collaborator_cannot_invite() {
        let e = SpindleEnforcer::new().await.unwrap();

        e.add_repo("did:plc:alice", "my-repo").await.unwrap();
        e.add_collaborator("did:plc:alice", "my-repo", "did:plc:bob")
            .await
            .unwrap();

        // Bob is a collaborator, not an owner — should not be able to invite
        assert!(
            !e.is_collaborator_invite_allowed("did:plc:bob", "did:plc:alice", "my-repo")
                .await
                .unwrap()
        );
    }
}
