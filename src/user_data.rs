use getrandom::SysRng;
use rand::{RngExt, rand_core::UnwrapErr};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserData {
    pub current: usize,
    pub change_nickname: bool,
    pub pronouns: Vec<String>,
}

impl UserData {
    pub fn current_pronoun(&self) -> &str {
        &self.pronouns[self.current]
    }

    pub fn current_and_reroll(&mut self) -> String {
        let prev = self.current_pronoun().to_string();
        self.reroll();
        prev
    }

    pub fn reroll(&mut self) {
        self.current = if self.pronouns.len() < 2 {
            0
        } else {
            UnwrapErr(SysRng).random_range(0..self.pronouns.len())
        };
    }

    pub fn current_sanity(&mut self) {
        if self.current >= self.pronouns.len() {
            self.reroll();
        }
    }
}
