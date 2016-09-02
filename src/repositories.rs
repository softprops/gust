//! Repository interface
extern crate serde_json;

use self::super::{Github, Result};
use deployments::Deployments;
use keys::Keys;
use issues::{IssueRef, Issues};
use labels::Labels;
use pulls::PullRequests;
use releases::Releases;
use rep::{Repo, RepoOptions, RepoListOptions, UserRepoListOptions, OrganizationRepoListOptions};
use statuses::Statuses;
use collaborators::Collaborators;
use std::fmt;

/// describes repository visibilities
#[derive(Clone, Debug, PartialEq)]
pub enum Visibility {
    All,
    Public,
    Private,
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "{}",
               match *self {
                   Visibility::All => "all",
                   Visibility::Public => "public",
                   Visibility::Private => "private",
               })
    }
}

/// Describes sorting options for repositories
#[derive(Clone, Debug, PartialEq)]
pub enum Sort {
    Created,
    Updated,
    Pushed,
    FullName,
}

impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "{}",
               match *self {
                   Sort::Created => "created",
                   Sort::Updated => "updated",
                   Sort::Pushed => "pushed",
                   Sort::FullName => "full_name",
               })
    }
}

/// Describes member affiliation types for repositories
#[derive(Clone, Debug, PartialEq)]
pub enum Affiliation {
    Owner,
    Collaborator,
    OrganizationMember,
}

impl fmt::Display for Affiliation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "{}",
               match *self {
                   Affiliation::Owner => "owner",
                   Affiliation::Collaborator => "collaborator",
                   Affiliation::OrganizationMember => "organization_member",
               })
    }
}

/// Describes types of repositories
#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    All,
    Owner,
    Public,
    Private,
    Member,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "{}",
               match *self {
                   Type::All => "all",
                   Type::Owner => "owner",
                   Type::Public => "public",
                   Type::Private => "private",
                   Type::Member => "member",
               })
    }
}

/// Describes types of organization repositories
#[derive(Clone, Debug, PartialEq)]
pub enum OrgRepoType {
    All,
    Public,
    Private,
    Forks,
    Sources,
    Member,
}

impl fmt::Display for OrgRepoType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "{}",
               match *self {
                   OrgRepoType::All => "all",
                   OrgRepoType::Public => "public",
                   OrgRepoType::Private => "private",
                   OrgRepoType::Forks => "forks",
                   OrgRepoType::Sources => "sources",
                   OrgRepoType::Member => "member",
               })
    }
}

pub struct Repositories<'a> {
    github: &'a Github<'a>,
}

impl<'a> Repositories<'a> {
    pub fn new(github: &'a Github<'a>) -> Repositories<'a> {
        Repositories { github: github }
    }

    fn path(&self, more: &str) -> String {
        format!("/user/repos{}", more)
    }

    /// Create a new repository
    /// https://developer.github.com/v3/repos/#create
    pub fn create(&self, repo: &RepoOptions) -> Result<Repo> {
        let data = try!(serde_json::to_string(&repo));
        self.github.post::<Repo>(&self.path(""), data.as_bytes())
    }

    /// list the authenticated users repositories
    /// https://developer.github.com/v3/repos/#list-your-repositories
    pub fn list(&self, options: &RepoListOptions) -> Result<Vec<Repo>> {
        let mut uri = vec![self.path("")];
        if let Some(query) = options.serialize() {
            uri.push(query);
        }
        self.github.get::<Vec<Repo>>(&uri.join("?"))
    }
}

/// Provides access to the authenticated user's repositories
pub struct UserRepositories<'a> {
    github: &'a Github<'a>,
    owner: String,
}

impl<'a> UserRepositories<'a> {
    pub fn new<O>(github: &'a Github<'a>, owner: O) -> UserRepositories<'a>
        where O: Into<String>
    {
        UserRepositories {
            github: github,
            owner: owner.into(),
        }
    }

    fn path(&self, more: &str) -> String {
        format!("/users/{}/repos{}", self.owner, more)
    }

    /// https://developer.github.com/v3/repos/#list-user-repositories
    pub fn list(&self, options: &UserRepoListOptions) -> Result<Vec<Repo>> {
        let mut uri = vec![self.path("")];
        if let Some(query) = options.serialize() {
            uri.push(query);
        }
        self.github.get::<Vec<Repo>>(&uri.join("?"))
    }
}

/// Provides access to an organization's repositories
pub struct OrganizationRepositories<'a> {
    github: &'a Github<'a>,
    org: String,
}

impl<'a> OrganizationRepositories<'a> {
    pub fn new<O>(github: &'a Github<'a>, org: O) -> OrganizationRepositories<'a>
        where O: Into<String>
    {
        OrganizationRepositories {
            github: github,
            org: org.into(),
        }
    }

    fn path(&self, more: &str) -> String {
        format!("/orgs/{}/repos{}", self.org, more)
    }

    /// https://developer.github.com/v3/repos/#list-organization-repositories
    pub fn list(&self, options: &OrganizationRepoListOptions) -> Result<Vec<Repo>> {
        let mut uri = vec![self.path("")];
        if let Some(query) = options.serialize() {
            uri.push(query);
        }
        self.github.get::<Vec<Repo>>(&uri.join("?"))
    }
}

pub struct Repository<'a> {
    github: &'a Github<'a>,
    owner: String,
    repo: String,
}

impl<'a> Repository<'a> {
    pub fn new<O, R>(github: &'a Github<'a>, owner: O, repo: R) -> Repository<'a>
        where O: Into<String>,
              R: Into<String>
    {
        Repository {
            github: github,
            owner: owner.into(),
            repo: repo.into(),
        }
    }

    /// get a reference to [deployments](https://developer.github.com/v3/repos/deployments/)
    /// associated with this repository ref
    pub fn deployments(&self) -> Deployments {
        Deployments::new(self.github, self.owner.as_str(), self.repo.as_str())
    }

    /// get a reference to a specific github issue associated with this repoistory ref
    pub fn issue(&self, number: u64) -> IssueRef {
        IssueRef::new(self.github, self.owner.as_str(), self.repo.as_str(), number)
    }

    /// get a reference to github issues associated with this repoistory ref
    pub fn issues(&self) -> Issues {
        Issues::new(self.github, self.owner.as_str(), self.repo.as_str())
    }

    /// get a reference to [deploy keys](https://developer.github.com/v3/repos/keys/)
    /// associated with this repository ref
    pub fn keys(&self) -> Keys {
        Keys::new(self.github, self.owner.as_str(), self.repo.as_str())
    }

    /// get a list of labels associated with this repository ref
    pub fn labels(&self) -> Labels {
        Labels::new(self.github, self.owner.as_str(), self.repo.as_str())
    }

    /// get a list of [pulls](https://developer.github.com/v3/pulls/)
    /// associated with this repository ref
    pub fn pulls(&self) -> PullRequests {
        PullRequests::new(self.github, self.owner.as_str(), self.repo.as_str())
    }

    /// get a reference to [releases](https://developer.github.com/v3/repos/releases/)
    /// associated with this repository ref
    pub fn releases(&self) -> Releases {
        Releases::new(self.github, self.owner.as_str(), self.repo.as_str())
    }

    /// get a references to [statuses](https://developer.github.com/v3/repos/statuses/)
    /// associated with this reposoitory ref
    pub fn statuses(&self) -> Statuses {
        Statuses::new(self.github, self.owner.as_str(), self.repo.as_str())
    }

    /// get a reference to the [collaborators](https://developer.github.com/v3/repos/collaborators/)
    /// associated with this repository ref
    pub fn collaborators(&self) -> Collaborators {
        Collaborators::new(self.github, self.owner.as_ref(), self.repo.as_ref())
    }
}
