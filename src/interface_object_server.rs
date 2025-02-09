struct InterfaceObjectServer {}

struct InterfaceObject {}

struct Property {}

#[reprt(u16)]
enum PidObjectType {
    ObjectType = 1,
    ObjectName = 2,
    LoadStateControl = 5,
}

struct PropertyIdentifier {}

struct PropertyValue {}
