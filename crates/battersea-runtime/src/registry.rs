use crate::{ExecutionHost, NodeHandler};
use battersea_flow::FlowNodeDefinition;
use std::collections::HashMap;
use std::sync::Arc;

pub struct RegistryBuilder<H: ExecutionHost> {
    handlers: HashMap<String, Arc<dyn NodeHandler<H>>>,
}
impl<H: ExecutionHost> Default for RegistryBuilder<H> {
    fn default() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }
}
impl<H: ExecutionHost> RegistryBuilder<H> {
    pub fn register(&mut self, handler: impl NodeHandler<H> + 'static) -> Result<(), String> {
        let id = handler.handler_id().to_string();
        if id.trim().is_empty() || self.handlers.contains_key(&id) {
            return Err(format!("Duplicate or empty handler id: {id}"));
        }
        self.handlers.insert(id, Arc::new(handler));
        Ok(())
    }
    /// Register a code-defined handler and return its metadata for catalogue composition.
    /// Catalogue validation is performed when the combined definitions are assembled.
    pub fn register_defined<
        T: NodeHandler<H> + battersea_flow::catalog::NodeDefinition + 'static,
    >(
        &mut self,
        handler: T,
        registry: &battersea_flow::registry::Registry,
    ) -> Result<FlowNodeDefinition, String> {
        let definition = T::definition(registry)?;
        if definition.handler_id != handler.handler_id() {
            return Err(format!(
                "Definition handler {:?} differs from runtime handler {:?}",
                definition.handler_id,
                handler.handler_id()
            ));
        }
        self.register(handler)?;
        Ok(definition)
    }
    pub fn build(self) -> HandlerRegistry<H> {
        HandlerRegistry {
            handlers: self.handlers,
        }
    }
}
pub struct HandlerRegistry<H: ExecutionHost> {
    handlers: HashMap<String, Arc<dyn NodeHandler<H>>>,
}
impl<H: ExecutionHost> HandlerRegistry<H> {
    pub fn get(&self, id: &str) -> Result<&dyn NodeHandler<H>, String> {
        self.handlers
            .get(id)
            .map(|h| h.as_ref())
            .ok_or_else(|| format!("No runtime handler is registered for \"{id}\"."))
    }
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.handlers.keys().map(String::as_str)
    }
    pub fn validate<'a>(
        &self,
        definitions: impl IntoIterator<Item = &'a FlowNodeDefinition>,
    ) -> Result<(), String> {
        for d in definitions {
            self.get(&d.handler_id)?;
        }
        Ok(())
    }
}

/// A catalogue whose declared handlers all resolve in an immutable registry.
pub struct ExecutableCatalog {
    catalog: battersea_flow::catalog::Catalog,
}
impl ExecutableCatalog {
    pub fn definitions(&self) -> &HashMap<String, FlowNodeDefinition> {
        self.catalog.definitions()
    }
    pub fn validate(
        &self,
        flow: &battersea_flow::FlowDocument,
    ) -> battersea_flow::FlowValidationResult {
        self.catalog.validate(flow)
    }
    pub fn contracts(&self) -> &battersea_flow::registry::Registry {
        self.catalog.registry()
    }
}
impl<H: ExecutionHost> HandlerRegistry<H> {
    pub fn bind_catalog(
        &self,
        catalog: battersea_flow::catalog::Catalog,
    ) -> Result<ExecutableCatalog, String> {
        self.validate(catalog.entries())?;
        Ok(ExecutableCatalog { catalog })
    }
}
