use std::convert::AsMut;
use std::marker::PhantomData;
use std::ops::Deref;

/// Fallback that is used when a type doesn't have any cli option.
pub struct NoDotNetCommandLineOption<O>(PhantomData<O>);
impl<O> NoDotNetCommandLineOption<O> {
    pub fn get_cli_option(&self) -> Option<O> {
        None
    }
}

/// Used by macro to check if a type implements `AsMut<Option<O>>` for a type
/// `O` that represents a CLI option.
pub struct MaybeDotNetCommandLineOption<'a, H, O> {
    holder: &'a mut H,
    option: NoDotNetCommandLineOption<O>,
}
impl<'a, H, O> MaybeDotNetCommandLineOption<'a, H, O> {
    pub fn new(holder: &'a mut H) -> Self {
        Self {
            holder,
            option: NoDotNetCommandLineOption(Default::default()),
        }
    }
}
impl<'a, H, O> MaybeDotNetCommandLineOption<'a, H, O>
where
    H: AsMut<Option<O>>,
{
    pub fn get_cli_option(&mut self) -> Option<O> {
        self.holder.as_mut().take()
    }
}
impl<'a, H, O> Deref for MaybeDotNetCommandLineOption<'a, H, O> {
    type Target = NoDotNetCommandLineOption<O>;
    fn deref(&self) -> &Self::Target {
        &self.option
    }
}

/// Define a new command line option.
macro_rules! impl_dot_cli_option {
    ($name:ident, $flag:literal) => {
        impl DotNetCommandLineOption for $name {
            fn value(&self) -> &str {
                &self.0.as_ref()
            }
            fn flag() -> &'static str {
                $flag
            }
        }

        impl From<$name> for Cow<'static, str> {
            fn from(value: $name) -> Cow<'static, str> {
                value.0
            }
        }
        impl From<Cow<'static, str>> for $name {
            fn from(value: Cow<'static, str>) -> Self {
                Self(value)
            }
        }
        impl From<&'static str> for $name {
            fn from(value: &'static str) -> Self {
                Self(Cow::from(value))
            }
        }
        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(Cow::from(value))
            }
        }
    };
}

/// Define a new subcommand for `dotnet`.
macro_rules! define_command {
    ($dest_name:ident From() $value_name:ident => $value:expr ) => {};
    ($dest_name:ident From($src_name:ident $(, $( $src_token:tt )* )? ) $value_name:ident => $value:expr) => {
        impl From<$src_name> for $dest_name {
            fn from(mut $value_name: $src_name) -> Self {
                $value
            }
        }
        define_command!($dest_name From( $($( $src_token )*)? ) $value_name => $value);
    };
    (
        #[command = $cmd:literal]
        $(#[ $($token:tt)* ])*
        $visible:vis struct $name:ident {
            $( $field_vis:vis $field_name:ident: $field_type:ty ),* $(,)?
        }
        $(
            $(,)?
            From($( $from_name:ident ),*)
        )?
    ) => {
        $( #[ $( $token )* ] )*
        $visible struct $name {
            $(
                $field_vis $field_name: Option<$field_type>,
            )*
        }
        impl $name {
            pub fn args_iter(&self) -> impl Iterator<Item = &str> {
                iter::once($cmd)
                $(
                    .chain(create_arg_iter_from_cli_option(self.$field_name.as_ref()))
                )*
            }
        }
        impl DotNetCommand for $name {
            fn get_args<'a, R>(&'a self, f: impl FnOnce(&mut dyn Iterator<Item = &'a str>) -> R) -> R {
                f(&mut self.args_iter())
            }
        }
        $(
            impl AsMut<Option<$field_type>> for $name {
                fn as_mut(&mut self) -> &mut Option<$field_type> {
                    &mut self.$field_name
                }
            }
        )*
        define_command!(
            $name From($($(
                $from_name
            ),*)?)
            value => {
                Self {
                    $(
                        $field_name: helper_macros::MaybeDotNetCommandLineOption::<_, $field_type>::new(&mut value).get_cli_option(),
                    )*
                }
            }
        );
    };
}

/// Implement a setter for `DotNetInvoker` if the wrapped subcommand supports
/// the `cli_option` type.
macro_rules! setter {
    ($name:ident, $cli_option:ty) => {
        impl<C> DotNetInvoker<C>
        where
            C: AsMut<Option<$cli_option>>,
        {
            pub fn $name(mut self, value: impl Into<$cli_option>) -> Self {
                *self.command_data.as_mut() = Some(value.into());
                self
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_specialization_hack() {
        #[derive(Debug, PartialEq)]
        struct CliOption;

        #[derive(Debug, PartialEq)]
        struct Holder(Option<CliOption>);
        impl AsMut<Option<CliOption>> for Holder {
            fn as_mut(&mut self) -> &mut Option<CliOption> {
                &mut self.0
            }
        }
        assert_eq!(
            Some(CliOption),
            MaybeDotNetCommandLineOption::new(&mut Holder(Some(CliOption))).get_cli_option()
        )
    }
    #[test]
    fn macro_specialization_hack_fallback() {
        #[derive(Debug, PartialEq)]
        struct CliOption;

        #[derive(Debug, PartialEq)]
        struct Holder;
        assert_eq!(
            Option::<CliOption>::None,
            MaybeDotNetCommandLineOption::<_, CliOption>::new(&mut Holder).get_cli_option()
        )
    }
}
