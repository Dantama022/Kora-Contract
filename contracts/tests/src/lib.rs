//! Shared cross-contract test harness for multi-actor scenarios.
//!
//! Provides builder-style helpers for constructing realistic fixtures with
//! several verifiers (differing scores), several signers (partial
//! availability), and several SMEs (concurrent invoices). The goal is that
//! new tests can express multi-actor setup in a few lines instead of
//! reimplementing it ad hoc.
//!
//! ```ignore
//! let env = Env::default();
//! let world = TestWorld::builder(&env)
//!     .verifiers(3)
//!     .signers(4)
//!     .smes(2)
//!     .build();
//! assert_eq!(world.verifiers.len(), 3);
//! ```

#![cfg(test)]

extern crate std;

use soroban_sdk::{contracttype, testutils::Address as _, Address, Env, Vec};

/// A single verifier actor with an associated score.
#[contracttype]
#[derive(Clone)]
pub struct Verifier {
    pub address: Address,
    pub score: u32,
}

/// A single signer actor with an availability flag.
#[contracttype]
#[derive(Clone)]
pub struct Signer {
    pub address: Address,
    /// Whether this signer is currently available to sign.
    pub available: bool,
}

/// A single SME actor with a set of concurrent invoice ids.
#[contracttype]
#[derive(Clone)]
pub struct Sme {
    pub address: Address,
    pub invoice_ids: Vec<u64>,
}

/// A fully constructed multi-actor fixture.
pub struct TestWorld {
    pub admin: Address,
    pub verifiers: Vec<Verifier>,
    pub signers: Vec<Signer>,
    pub smes: Vec<Sme>,
}

impl TestWorld {
    /// Start building a fixture against the given environment.
    pub fn builder(env: &Env) -> TestWorldBuilder {
        TestWorldBuilder::new(env)
    }

    /// Addresses of all verifiers, in construction order.
    pub fn verifier_addresses(&self) -> Vec<Address> {
        let mut out = Vec::new(&self.verifiers.env());
        for v in self.verifiers.iter() {
            out.push_back(v.address.clone());
        }
        out
    }

    /// Addresses of signers that are currently available.
    pub fn available_signers(&self) -> Vec<Address> {
        let mut out = Vec::new(&self.signers.env());
        for s in self.signers.iter() {
            if s.available {
                out.push_back(s.address.clone());
            }
        }
        out
    }

    /// Addresses of signers that are currently unavailable.
    pub fn unavailable_signers(&self) -> Vec<Address> {
        let mut out = Vec::new(&self.signers.env());
        for s in self.signers.iter() {
            if !s.available {
                out.push_back(s.address.clone());
            }
        }
        out
    }

    /// Total number of invoices across all SMEs.
    pub fn total_invoices(&self) -> u32 {
        let mut total = 0u32;
        for sme in self.smes.iter() {
            total += sme.invoice_ids.len();
        }
        total
    }
}

/// Builder for [`TestWorld`].
///
/// Defaults are intentionally small (1 verifier, 1 signer, 1 SME) so that
/// single-actor tests remain terse while multi-actor tests scale up with a
/// single call per actor group.
pub struct TestWorldBuilder<'a> {
    env: &'a Env,
    admin: Option<Address>,
    verifier_count: u32,
    verifier_scores: Option<Vec<u32>>,
    signer_count: u32,
    unavailable_signers: u32,
    sme_count: u32,
    invoices_per_sme: u32,
}

impl<'a> TestWorldBuilder<'a> {
    pub fn new(env: &'a Env) -> Self {
        Self {
            env,
            admin: None,
            verifier_count: 1,
            verifier_scores: None,
            signer_count: 1,
            unavailable_signers: 0,
            sme_count: 1,
            invoices_per_sme: 1,
        }
    }

    /// Override the admin address (defaults to a freshly generated one).
    pub fn admin(mut self, admin: Address) -> Self {
        self.admin = Some(admin);
        self
    }

    /// Number of verifiers to generate.
    pub fn verifiers(mut self, count: u32) -> Self {
        self.verifier_count = count;
        self
    }

    /// Explicit scores for each verifier. Length must match `verifiers(n)`.
    pub fn verifier_scores(mut self, scores: &[u32]) -> Self {
        let mut v = Vec::new(self.env);
        for s in scores.iter() {
            v.push_back(*s);
        }
        self.verifier_scores = Some(v);
        self
    }

    /// Number of signers to generate.
    pub fn signers(mut self, count: u32) -> Self {
        self.signer_count = count;
        self
    }

    /// Mark the first `n` signers as unavailable (partial availability).
    pub fn unavailable_signers(mut self, n: u32) -> Self {
        self.unavailable_signers = n;
        self
    }

    /// Number of SMEs to generate.
    pub fn smes(mut self, count: u32) -> Self {
        self.sme_count = count;
        self
    }

    /// Number of concurrent invoices to assign to each SME.
    pub fn invoices_per_sme(mut self, count: u32) -> Self {
        self.invoices_per_sme = count;
        self
    }

    /// Materialize the fixture.
    pub fn build(self) -> TestWorld {
        let env = self.env;
        let admin = self.admin.unwrap_or_else(|| Address::generate(env));

        let mut verifiers = Vec::new(env);
        for i in 0..self.verifier_count {
            let score = match &self.verifier_scores {
                Some(scores) => scores.get(i).unwrap_or(0),
                None => 50 + i * 10,
            };
            verifiers.push_back(Verifier {
                address: Address::generate(env),
                score,
            });
        }

        let mut signers = Vec::new(env);
        for i in 0..self.signer_count {
            signers.push_back(Signer {
                address: Address::generate(env),
                available: i >= self.unavailable_signers,
            });
        }

        let mut smes = Vec::new(env);
        let mut next_invoice_id: u64 = 1;
        for _ in 0..self.sme_count {
            let mut invoice_ids = Vec::new(env);
            for _ in 0..self.invoices_per_sme {
                invoice_ids.push_back(next_invoice_id);
                next_invoice_id += 1;
            }
            smes.push_back(Sme {
                address: Address::generate(env),
                invoice_ids,
            });
        }

        TestWorld {
            admin,
            verifiers,
            signers,
            smes,
        }
    }
}
