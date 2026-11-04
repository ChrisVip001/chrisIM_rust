use tonic::Status;

pub mod conversation;
mod friend;
mod group;
pub mod msg;
mod user;

pub trait Validator {
    fn validate(&self) -> Result<(), Status>;
}
