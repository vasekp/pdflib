use super::*;

/// This trait provides means of resolving indirect object references ([`ObjRef`]) into the 
/// actual [`Object`]s.
pub trait Resolver {
    /// Resolves an [`ObjRef`] into an owned [`Object`].
    fn resolve_ref(&self, objref: &ObjRef) -> Result<Object, Error>;

    /// For an [`Object::Ref`], calls [`Self::resolve_ref()`], otherwise returns `obj` unchanged.
    fn resolve_obj(&self, obj: Object) -> Result<Object, Error> {
        match obj {
            Object::Ref(objref) => self.resolve_ref(&objref),
            _ => Ok(obj)
        }
    }

    /// Resolves indirect references like [`Self::resolve_obj()`], but also traverses to 
    /// the first level in [`Object::Array`]s and [`Object::Dict`]s.
    fn resolve_deep(&self, obj: Object) -> Result<Object, Error> {
        Ok(match self.resolve_obj(obj)? {
            Object::Array(arr) =>
                Object::Array(arr.into_iter()
                    .map(|obj| self.resolve_obj(obj))
                    .collect::<Result<Vec<_>, _>>()?),
            Object::Dict(dict) =>
                Object::Dict(Dict::from(dict.into_inner()
                    .into_iter()
                    .map(|(name, obj)| -> Result<(Name, Object), Error> {
                        Ok((name, self.resolve_obj(obj)?))
                    })
                    .collect::<Result<Vec<_>, _>>()?)),
            obj => obj
        })
    }
}

impl Resolver for () {
    fn resolve_ref(&self, _: &ObjRef) -> Result<Object, Error> {
        Err(Error::Parse("no resolver provided for resolving object references"))
    }
}
