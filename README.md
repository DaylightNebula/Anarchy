# Anarchy
This is an experimental ECS crate that allows for the highest possible degree of concurrency
to an ECS world while systems run rampant through it.  This is acheived by only locking data
at the lowest point needed for a general purpose ECS system, this being generally at the component
or resource level through a variety of techniques.


### Testing
You may recognize that this resource seems rather sparse for what it is trying to do with
a lack of examples or some unit tests.  This crate is mostly tested through a game engine
that is being worked on in the background, however, as there is no profit potentional or 
any other incentive for me to keep this to myself, I wish to release it for the public to
see.