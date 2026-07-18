use std::sync::atomic::AtomicU32;

use crate::AsAny;

/// The bit position assigned to a component type within an entity's component mask.
pub type ComponentID = u32;

/// Global counter used to hand out the next unused `ComponentID`.
pub static NEXT_BIT_MASK: AtomicU32 = AtomicU32::new(0);

/// Defines each component types ID.
/// Used for sorting component lists.
pub trait ComponentMeta {
    /// Returns the bit position assigned to this component type, allocated once from
    /// `NEXT_BIT_MASK` and cached (see the `Component` derive macro).
    fn bit_mask() -> ComponentID;
}

/// Trait to define each usuable component
/// inside tables.
pub trait Component: AsAny {
    /// Returns the bit mask associated with this component.
    /// Used for sorting component lists and searching
    /// database trees.
    fn get_bit_mask(&self) -> ComponentID;
}

impl AsAny for Box<dyn Component> {
    fn as_any(&self) -> &dyn std::any::Any {
        (**self).as_any()
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        (**self).as_any_mut()
    }
}

/// Builds a component mask from a list of component IDs.
pub fn build_bit_mask(components: &[ComponentID]) -> Box<[u8]> {
    // default case if no components given
    if components.is_empty() { return Box::new([]); }

    let mut output = Vec::new();

    for id in components {
        let segment = (id / 8) as usize;
        let idx = (id % 8) as usize;
        output.resize(segment + 1, 0);
        output[segment] = output[segment] | (1 << idx);
    }

    return output.into_boxed_slice();
}

/// Check if the given component mask matches the given
/// bit mask.
pub fn bit_masks_match(
    components: &[u8],
    mask: &[u8]
) -> bool {
    // if mask is empty, the default case is true
    if mask.len() == 0 {
        return true;
    }

    // if components is empty, default to false
    if components.len() == 0 {
        return false;
    }

    let mut component_idx = 0;
    for mask_part in mask {
        // if not components for this part, fail if mask is not empty, otherwise, skip
        if component_idx >= components.len() {
            if *mask_part == 0 { continue }
            else { return false }
        }

        let component_part = &components[component_idx];

        // check mask
        if component_part & mask_part != *mask_part {
            return false
        }

        component_idx += 1;
    }

    // if we made it this far, we succeeded
    return true;
}

/// Incrementally builds a component bit mask (as used by `build_bit_mask` and
/// `bit_masks_match`) by inserting one component ID or type at a time.
#[derive(Default, Debug, Clone)]
pub struct MaskBuilder(Vec<u8>);

impl MaskBuilder {
    /// Creates a new, empty `MaskBuilder`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the bit for the given raw component ID.
    pub fn insert_raw(&mut self, id: ComponentID) {
        let segment = (id / 8) as usize;
        let idx = (id % 8) as usize;
        self.0.resize(segment + 1, 0);
        self.0[segment] = self.0[segment] | (1 << idx);
    }

    /// Sets the bit for the given component type.
    pub fn insert<C: ComponentMeta>(&mut self) {
        self.insert_raw(C::bit_mask());
    }

    /// Consumes this builder, returning the built mask.
    pub fn build(self) -> Box<[u8]> { self.0.into_boxed_slice() }
}

#[cfg(test)]
mod tests {
    use crate::ecs::components::{bit_masks_match, build_bit_mask};

    #[test]
    pub fn test_build_bit_mask_one() {
        assert!(build_bit_mask(&[]).len() == 0)
    }

    #[test]
    pub fn test_build_bit_mask_two() {
        let mask = &build_bit_mask(&[0, 1]);
        assert!(mask.len() == 1);
        assert!(mask[0] == 3);
    }

    #[test]
    pub fn test_comp_mask_one() {
        assert!(bit_masks_match(
            &build_bit_mask(&[0, 1, 2, 3]),
            &build_bit_mask(&[0, 1, 2])
        ))
    }

    #[test]
    pub fn test_comp_mask_two() {
        assert!(bit_masks_match(
            &build_bit_mask(&[0, 1, 2, 3]),
            &build_bit_mask(&[1, 2, 3])
        ))
    }

    #[test]
    pub fn test_comp_mask_three() {
        assert!(bit_masks_match(
            &build_bit_mask(&[0, 1, 2, 3]),
            &build_bit_mask(&[1, 2])
        ))
    }

    #[test]
    pub fn test_comp_mask_four() {
        let components = build_bit_mask(&[0, 1, 2, 3]);
        let mask = build_bit_mask(&[2]);

        assert!(components.len() == 1);
        assert!(mask.len() == 1);
        assert!(components[0] == 15);
        assert!(mask[0] == 4);

        assert!(bit_masks_match(
            &components,
            &mask
        ))
    }

    #[test]
    pub fn test_comp_mask_five() {
        assert!(bit_masks_match(
            &build_bit_mask(&[0, 1, 2, 3]),
            &build_bit_mask(&[])
        ))
    }

    #[test]
    pub fn test_comp_mask_six() {
        assert!(!bit_masks_match(
            &build_bit_mask(&[0, 1, 2, 3]),
            &build_bit_mask(&[1, 4])
        ))
    }

    #[test]
    pub fn test_comp_mask_seven() {
        assert!(!bit_masks_match(
            &build_bit_mask(&[0, 1, 2, 3]),
            &build_bit_mask(&[4])
        ))
    }
}
