# Anarchy V2
This is an experimental ECS crate that allows for the highest possible degree of concurrency
to an ECS world while systems run rampant through it.  This is acheived by only locking data
at the lowest point needed for a general purpose ECS system, this being generally at the component
or resource level through a variety of techniques.  This also enables faster data processing through
queries that allow work to be shared across threads.